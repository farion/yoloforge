use super::*;
use axum::{extract::State, routing::get, Json, Router};
use clap::Parser;
use serde_json::json;
use std::sync::{Arc, Mutex};

async fn fetch_control(
    State(state): State<Arc<Mutex<StreamControl>>>,
) -> Json<Option<StreamControl>> {
    Json(Some(state.lock().unwrap().clone()))
}
async fn report_control(
    State(state): State<Arc<Mutex<StreamControl>>>,
    Json(body): Json<ReportControl>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let mut control = state.lock().unwrap();
    assert_eq!(body.control_request, control.request_id);
    let lose_response = control.state == PauseState::Requested;
    assert_eq!(body.ownership_epoch, 1);
    control.state = body.state;
    control.summary = body.summary;
    control.proof = body.proof;
    if lose_response {
        axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response()
    } else {
        Json(control.clone()).into_response()
    }
}

struct Cleanup(Config);

#[tokio::test(flavor = "multi_thread")]
async fn replaced_control_during_report_is_reconciled_without_exiting_runner() {
    let dir = tempfile::tempdir().unwrap();
    let runner = RunnerId::new();
    let command = RunnerCommand {
        id: CommandId::new(),
        stream_id: StreamId::new(),
        runner_id: runner,
        ownership_epoch: 1,
        kind: CommandKind::RunAgent,
        state: CommandState::Accepted,
        result: None,
        created_unix_ms: 1,
        workspace_spec: None,
        agent_spec: None,
        agent_progress: None,
    };
    let original = StreamControl {
        retry: None,
        action: ControlAction::Resume,
        proof: None,
        stream_id: command.stream_id,
        request_id: RequestId::new(),
        command_id: Some(command.id),
        state: PauseState::ResumeRequested,
        summary: "retry race".into(),
        previous_status: StreamState::Running,
    };
    let state = Arc::new(Mutex::new(original.clone()));
    let router=Router::new().route("/v1/runners/{runner}/commands/{id}/control",get(fetch_control).post(|State(state):State<Arc<Mutex<StreamControl>>>|async move {
        state.lock().unwrap().request_id=RequestId::new();
        (axum::http::StatusCode::CONFLICT,Json(json!({"code":"work_conflict","message":"Superseded request","request_id":RequestId::new(),"current_revision":null})))
    })).with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let factory =
        FactoryClient::new(&endpoint, "test-only-factory-token-0123456789abcdef").unwrap();
    let journal = Journal::open(&dir.path().join("journal"), &endpoint, runner).unwrap();
    let mut record = journal.receive(command.clone()).unwrap();
    record.agent=Some(serde_json::from_value(json!({"runtime":{"config":{"stream_id":command.stream_id,"container_id":"a".repeat(64),"docker_socket":"/missing"},"started_at":"old","endpoint":"http://127.0.0.1:1","workspace":"/workspace"},"workspace":{"path":"/workspace","branch":"test","base_commit":"a".repeat(40)},"endpoint":"http://127.0.0.1:1","session_attempted":true,"prompt_attempted":true})).unwrap());
    // Missing configuration yields an unknown report, whose request is replaced
    // while in flight. It must not trigger writes or a fatal runner conflict.
    let args = Args::try_parse_from([
        "factory-runner",
        "--id",
        &runner.to_string(),
        "--name",
        "race",
    ])
    .unwrap();
    assert!(super::control(
        &factory,
        &journal,
        &mut record,
        &args,
        RegistrationId::new()
    )
    .await
    .unwrap());
    assert_ne!(state.lock().unwrap().request_id, original.request_id);
    assert!(record.container_actions.is_empty());
    server.abort();
}
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::process::Command::new("docker")
            .arg("--host")
            .arg(format!("unix://{}", self.0.docker_socket.display()))
            .args(["rm", "-f", &self.0.container_id])
            .output();
    }
}
fn sample(workspace: &Path) -> Vec<u8> {
    ["parent", "detached"]
        .iter()
        .flat_map(|p| std::fs::read(workspace.join(p)).unwrap_or_default())
        .collect()
}

/// Explicit opt-in: owns only a fresh labelled test container, never the shared OpenCode lab.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires local Docker cgroup v2 and yoloforge-freezer-test:development image"]
async fn real_freezer_stops_detached_signal_resistant_tools_and_reconciles_after_restart() {
    exercise_runtime(false).await;
}
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires local Docker cgroup v2 and yoloforge-freezer-test:development image"]
async fn real_freezer_removal_stops_detached_writers_and_replays_evidence() {
    exercise_runtime(true).await;
}
async fn exercise_runtime(terminate: bool) {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::Builder::new()
        .prefix("freezer-test-")
        .tempdir_in("/tmp/opencode")
        .unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    let work = dir.path().join("work");
    std::fs::create_dir(&work).unwrap();
    std::fs::set_permissions(&work, std::fs::Permissions::from_mode(0o777)).unwrap();
    let runner = RunnerId::new();
    let stream = StreamId::new();
    let mut c = Config {
        stream_id: stream,
        container_id: String::new(),
        docker_socket: std::fs::canonicalize("/var/run/docker.sock").unwrap(),
    };
    // One ordinary child and one double-forked, setsid child ignoring termination signals.
    // PID 1 stays alive, so daemon observation alone cannot prove the tools are stopped.
    let code = format!(
        r#"import os,signal,time
def pulse(name):
    signal.signal(signal.SIGTERM,signal.SIG_IGN)
    signal.signal(signal.SIGHUP,signal.SIG_IGN)
    signal.signal(signal.SIGINT,signal.SIG_IGN)
    while True:
        with open({:?}+'/'+name,'ab',buffering=0) as f: f.write(b'x')
        time.sleep(.03)
if os.fork()==0:
    os.setsid()
    if os.fork()!=0: os._exit(0)
    pulse('detached')
if os.fork()==0: pulse('parent')
while True: time.sleep(1)
"#,
        work.to_str().unwrap()
    );
    let mount = format!(
        "type=bind,source={},target={}",
        work.display(),
        work.display()
    );
    let label_stream = format!("io.yoloforge.stream={stream}");
    let label_runner = format!("io.yoloforge.runner={runner}");
    let id = docker(
        &c,
        &[
            "create",
            "--read-only",
            "--user",
            "10001:10001",
            "--cap-drop",
            "ALL",
            "--security-opt",
            "no-new-privileges",
            "--cgroupns",
            "private",
            "--ipc",
            "private",
            "--restart",
            "no",
            "--label",
            &label_stream,
            "--label",
            &label_runner,
            "--mount",
            &mount,
            "-p",
            "127.0.0.1::4096",
            "yoloforge-freezer-test:development",
            "python3",
            "-c",
            &code,
        ],
    )
    .await
    .unwrap();
    c.container_id = std::str::from_utf8(&id).unwrap().trim().into();
    let _cleanup = Cleanup(c.clone());
    docker(&c, &["start", &c.container_id]).await.unwrap();
    let raw: Vec<Value> =
        serde_json::from_slice(&docker(&c, &["inspect", &c.container_id]).await.unwrap()).unwrap();
    let port = raw[0]["NetworkSettings"]["Ports"]["4096/tcp"][0]["HostPort"]
        .as_str()
        .unwrap();
    let endpoint = format!("http://127.0.0.1:{port}");
    let config_path = dir.path().join("containment.json");
    std::fs::write(&config_path, serde_json::to_vec(&c).unwrap()).unwrap();
    let registration = dir.path().join("opencode.json");
    std::fs::write(
        &registration,
        json!({"url":endpoint,"password":"test-only"}).to_string(),
    )
    .unwrap();
    let args = Args::try_parse_from([
        "factory-runner",
        "--id",
        &runner.to_string(),
        "--name",
        "freezer-test",
        "--containment",
        config_path.to_str().unwrap(),
        "--opencode-registration",
        registration.to_str().unwrap(),
    ])
    .unwrap();
    let command = RunnerCommand {
        id: CommandId::new(),
        stream_id: stream,
        runner_id: runner,
        ownership_epoch: 1,
        kind: CommandKind::RunAgent,
        state: CommandState::Accepted,
        result: None,
        created_unix_ms: 1,
        workspace_spec: None,
        agent_spec: None,
        agent_progress: None,
    };
    let runtime = bind(&args, &command, &endpoint, work.to_str().unwrap())
        .await
        .unwrap()
        .unwrap();
    let journal_path = dir.path().join("runner.db");
    let journal = Journal::open(&journal_path, &args.server, runner).unwrap();
    let mut record = journal.receive(command.clone()).unwrap();
    record.agent=Some(serde_json::from_value(json!({"runtime":runtime,"workspace":{"path":work,"branch":"test","base_commit":"a".repeat(40)},"endpoint":endpoint,"session_attempted":true,"prompt_attempted":true})).unwrap());
    journal.save(&record).unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(std::fs::metadata(work.join("detached")).unwrap().len() > 0);
    let mut control = StreamControl {
        retry: None,
        action: ControlAction::Pause,
        proof: None,
        stream_id: stream,
        request_id: RequestId::new(),
        command_id: Some(command.id),
        state: PauseState::Requested,
        summary: "pause".into(),
        previous_status: StreamState::Running,
    };
    let state = Arc::new(Mutex::new(control.clone()));
    let router = Router::new()
        .route(
            "/v1/runners/{runner}/commands/{id}/control",
            get(fetch_control).post(report_control),
        )
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let factory = FactoryClient::new(
        &format!("http://{}", listener.local_addr().unwrap()),
        "test-only-factory-token-0123456789abcdef",
    )
    .unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    // There is no OpenCode HTTP listener in this fixture. The control path must
    // operate entirely via Factory and Docker even while the runtime is frozen.
    assert!(super::control(
        &factory,
        &journal,
        &mut record,
        &args,
        RegistrationId::new()
    )
    .await
    .is_err());
    assert!(super::control(
        &factory,
        &journal,
        &mut record,
        &args,
        RegistrationId::new()
    )
    .await
    .unwrap());
    assert_eq!(state.lock().unwrap().state, PauseState::Frozen);
    let frozen = state.lock().unwrap().proof.clone().unwrap();
    assert!(frozen.frozen);
    let before = sample(&work);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(sample(&work), before);
    assert!(ready(&record, &args).await.is_err());
    drop(journal);
    let journal = Journal::open(&journal_path, &args.server, runner).unwrap();
    let mut record = journal.record(command.id).unwrap().unwrap();
    assert!(
        transition(&journal, &mut record, &args, &control)
            .await
            .unwrap()
            .frozen
    );
    // External drift must not cause a repeated pause, including after journal restart.
    docker(&c, &["unpause", &c.container_id]).await.unwrap();
    assert!(transition(&journal, &mut record, &args, &control)
        .await
        .is_err());
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(sample(&work).len() > before.len());
    docker(&c, &["pause", &c.container_id]).await.unwrap();
    control.action = ControlAction::Resume;
    control.state = PauseState::ResumeRequested;
    control.request_id = RequestId::new();
    control.proof = Some(frozen.clone());
    control.proof.as_mut().unwrap().container_id = "b".repeat(64);
    assert!(transition(&journal, &mut record, &args, &control)
        .await
        .is_err());
    assert!(
        current(&record, &args).await.unwrap().unwrap()["State"]["Paused"]
            .as_bool()
            .unwrap()
    );
    control.proof = Some(frozen);
    assert!(
        !transition(&journal, &mut record, &args, &control)
            .await
            .unwrap()
            .frozen
    );
    ready(&record, &args).await.unwrap();
    let before = sample(&work);
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(sample(&work).len() > before.len());
    // Crash after persisting intent, before Docker call: no automatic retry.
    control.action = ControlAction::Pause;
    control.state = PauseState::Requested;
    control.proof = None;
    control.request_id = RequestId::new();
    record.container_actions.insert(
        control.request_id,
        Attempt {
            action: ControlAction::Pause,
        },
    );
    journal.save(&record).unwrap();
    drop(journal);
    let journal = Journal::open(&journal_path, &args.server, runner).unwrap();
    let mut record = journal.record(command.id).unwrap().unwrap();
    assert!(transition(&journal, &mut record, &args, &control)
        .await
        .is_err());
    docker(&c, &["pause", &c.container_id]).await.unwrap();
    assert!(
        transition(&journal, &mut record, &args, &control)
            .await
            .unwrap()
            .frozen
    );
    // An operator-authorized *new* thaw request may recover an intent that was
    // persisted before a crash, without erasing or replaying the old receipt.
    control.proof = Some(
        transition(&journal, &mut record, &args, &control)
            .await
            .unwrap(),
    );
    control.action = ControlAction::Resume;
    control.state = PauseState::ResumeRequested;
    control.request_id = RequestId::new();
    let old_request = control.request_id;
    record.container_actions.insert(
        old_request,
        Attempt {
            action: ControlAction::Resume,
        },
    );
    journal.save(&record).unwrap();
    drop(journal);
    let journal = Journal::open(&journal_path, &args.server, runner).unwrap();
    let mut record = journal.record(command.id).unwrap().unwrap();
    assert!(transition(&journal, &mut record, &args, &control)
        .await
        .is_err());
    control.request_id = RequestId::new();
    control.retry = Some(ControlRetry {
        previous_request: old_request,
        reason: "Explicit operator recovery".into(),
    });
    assert!(
        !transition(&journal, &mut record, &args, &control)
            .await
            .unwrap()
            .frozen
    );
    assert!(record.container_actions.contains_key(&old_request));
    assert!(record.container_actions.contains_key(&control.request_id));
    if terminate {
        let saved = record
            .agent
            .as_ref()
            .unwrap()
            .runtime
            .as_ref()
            .unwrap()
            .clone();
        let mut request = RuntimeTermination {
            automatic_failover: false,
            binding: None,
            request_id: RequestId::new(),
            command_id: command.id,
            stream_id: stream,
            runner_id: runner,
            ownership_epoch: 1,
            runtime: RuntimeRef {
                container_id: c.container_id.clone(),
                started_at: saved.started_at.clone(),
            },
            previous_request: None,
            reason: "Test removal".into(),
            state: TerminationState::Requested,
            summary: String::new(),
            proof: None,
            result: None,
            archive_may_be_incomplete: true,
        };
        // A foreign engine or incarnation never authorizes a destructive write.
        record
            .agent
            .as_mut()
            .unwrap()
            .runtime
            .as_mut()
            .unwrap()
            .daemon_id = Some("foreign-engine".into());
        assert!(remove(&journal, &mut record, &args, &request)
            .await
            .is_err());
        record.agent.as_mut().unwrap().runtime = Some(saved);
        request.runtime.started_at = "foreign-incarnation".into();
        assert!(remove(&journal, &mut record, &args, &request)
            .await
            .is_err());
        request.runtime.started_at = record
            .agent
            .as_ref()
            .unwrap()
            .runtime
            .as_ref()
            .unwrap()
            .started_at
            .clone();
        // Intent persisted but never executed: reconciliation may not repeat rm.
        record.removals.insert(
            request.request_id,
            RemovalAttempt {
                runtime: request.runtime.clone(),
                proof: None,
            },
        );
        journal.save(&record).unwrap();
        assert!(remove(&journal, &mut record, &args, &request)
            .await
            .is_err());
        assert!(!absent(&c).await.unwrap());
        request.previous_request = Some(request.request_id);
        request.request_id = RequestId::new();
        let saved = record.agent.as_ref().unwrap().runtime.as_ref().unwrap();
        request.binding = Some(RuntimeBinding {
            command_id: command.id,
            stream_id: stream,
            runner_id: runner,
            ownership_epoch: 1,
            runtime: request.runtime.clone(),
            daemon_id: saved.daemon_id.clone().unwrap(),
            endpoint: saved.endpoint.clone(),
            workspace: saved.workspace.clone(),
        });
        // Recover removal from central evidence after losing the entire local
        // execution identity. Never recover a prompt or continuation this way.
        record.agent = None;
        docker(&c, &["pause", &c.container_id]).await.unwrap();
        let proof = remove(&journal, &mut record, &args, &request)
            .await
            .unwrap();
        assert!(absent(&c).await.unwrap());
        let before = sample(&work);
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(before, sample(&work));
        assert!(!before.is_empty());
        drop(journal);
        let journal = Journal::open(&journal_path, &args.server, runner).unwrap();
        let mut record = journal.record(command.id).unwrap().unwrap();
        assert_eq!(
            remove(&journal, &mut record, &args, &request)
                .await
                .unwrap(),
            proof
        );
        // Simulate loss between removal and proof persistence: absence on the
        // pinned daemon is enough, with no second destructive operation.
        record.removals.get_mut(&request.request_id).unwrap().proof = None;
        journal.save(&record).unwrap();
        assert_eq!(
            remove(&journal, &mut record, &args, &request)
                .await
                .unwrap()
                .runtime,
            proof.runtime
        );
        server.abort();
        return;
    }
    docker(&c, &["restart", "--time", "0", &c.container_id])
        .await
        .unwrap();
    assert!(ready(&record, &args).await.is_err());
    assert!(transition(&journal, &mut record, &args, &control)
        .await
        .is_err());
    // Removing configuration or changing endpoint never adopts/rebinds the execution.
    let mut removed = args;
    removed.containment = None;
    assert!(ready(&record, &removed).await.is_err());
    // Validate fail-closed checks against the actual engine's inspect shape.
    for pointer in [
        "/HostConfig/Privileged",
        "/HostConfig/ReadonlyRootfs",
        "/HostConfig/PidMode",
        "/HostConfig/CgroupnsMode",
        "/HostConfig/CapDrop",
        "/Config/User",
        "/Config/Labels/io.yoloforge.stream",
    ] {
        let mut changed = raw[0].clone();
        *changed.pointer_mut(pointer).unwrap() = Value::Null;
        assert!(
            validate(&changed, &c, runner, &endpoint, work.to_str().unwrap()).is_err(),
            "accepted {pointer}"
        );
    }
    server.abort();
}
