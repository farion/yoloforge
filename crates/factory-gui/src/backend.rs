//! All network and blocking desktop operations stay off the egui thread.
use factory_core::*;
use factory_protocol::{
    client::{ClientError, FactoryClient},
    *,
};
use serde_json::Value;
use std::{path::PathBuf, sync::mpsc, time::Duration};

pub enum Event {
    Snapshot(Result<Snapshot, String>),
    SnapshotPeer(Option<std::net::IpAddr>),
    Instances(Result<Vec<OpenCodeInstance>, String>),
    Clients(Result<Vec<ClientPresence>, String>),
    Notifications(Result<NotificationPage, String>, bool),
    NotificationStates(Result<Vec<NotificationReadState>, String>),
    Events(Result<EventPage, String>),
    History(Result<EventHistoryPage, String>),
    Request(u64, Result<Value, Failure>),
    Archive(
        CommandId,
        u64,
        ArchiveRead,
        Result<Box<ArchiveWindow>, String>,
    ),
    Attachment(RequestId, Result<InputAttachment, String>),
    Preview(String, Result<egui::ColorImage, String>),
    Saved(Result<String, String>),
    Local(Result<crate::persistence::LocalConnection, String>),
    LocalRunners(crate::local_runners::Report),
    Access(Result<AccessView, String>),
    TaskPayload(TicketId, Revision, u64, Result<Ticket, String>),
    TaskHistory(RequestId, Option<u64>, Result<TaskHistoryPage, String>),
}

pub struct Envelope {
    pub generation: u64,
    pub event: Event,
}

#[derive(Clone, Debug)]
pub struct Failure {
    pub message: String,
    pub uncertain: bool,
}
impl From<ClientError> for Failure {
    fn from(error: ClientError) -> Self {
        let uncertain =
            !matches!(&error, ClientError::Http { status, .. } if status.is_client_error());
        Self {
            message: error.to_string(),
            uncertain,
        }
    }
}
impl From<factory_protocol::client::RequestFailure> for Failure {
    fn from(error: factory_protocol::client::RequestFailure) -> Self {
        let mut failure = Self::from(error.error);
        if let Some(message) = error.message.filter(|s| !s.trim().is_empty()) {
            failure.message = format!("{message} ({})", failure.message);
        }
        failure
    }
}

pub enum InputAttachment {
    Image(PromptImage),
    File(PromptFile),
}
#[derive(Clone, Copy, Debug)]
pub enum ArchiveRead {
    Recent,
    Newer(u64),
    Older(u64),
}

pub struct Backend {
    runtime: RuntimeGuard,
    tx: mpsc::Sender<Envelope>,
    pub rx: mpsc::Receiver<Envelope>,
    ctx: egui::Context,
    client: Option<FactoryClient>,
    pub(crate) desktop_id: Option<RequestId>,
    connection: Option<tokio::task::JoinHandle<()>>,
    pub generation: u64,
    storage_tx: tokio::sync::mpsc::UnboundedSender<StorageWrite>,
}
impl Backend {
    pub fn new(ctx: egui::Context) -> Self {
        let (tx, rx) = mpsc::channel();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(3)
            .enable_all()
            .build()
            .expect("Tokio runtime");
        let (storage_tx, mut storage_rx) = tokio::sync::mpsc::unbounded_channel::<StorageWrite>();
        let error_tx = tx.clone();
        let repaint = ctx.clone();
        runtime.spawn(async move {
            while let Some(write) = storage_rx.recv().await {
                let acknowledged = write.ack.is_some();
                let result = tokio::task::spawn_blocking(move || crate::persistence::save_settings(&write.settings)).await;
                if let Some(ack) = write.ack { let _ = ack.send(matches!(result, Ok(Ok(())))); }
                if let Some(request) = write.request {
                    if matches!(result, Ok(Ok(()))) {
                        tokio::spawn(async move { request.sender.send(Event::Request(request.id, perform_request(&request.client, &request.method, &request.path, &request.body).await)); });
                    } else {
                        request.sender.send(Event::Request(request.id, Err(Failure { message: crate::tr!("Nicht gesendet: lokales Anfragejournal konnte nicht gespeichert werden.").into(), uncertain: false })));
                    }
                } else if !acknowledged && !matches!(result, Ok(Ok(()))) {
                    let _ = error_tx.send(Envelope { generation: u64::MAX, event: Event::Saved(Err(crate::tr!("Lokales Speichern fehlgeschlagen. Entwurf vor dem Beenden sichern.").into())) }); repaint.request_repaint();
                }
            }
        });
        Self {
            runtime: RuntimeGuard(Some(runtime)),
            tx,
            rx,
            ctx,
            client: None,
            desktop_id: None,
            connection: None,
            generation: 0,
            storage_tx,
        }
    }
    pub fn persist(&self, settings: crate::persistence::Settings) {
        let _ = self.storage_tx.send(StorageWrite {
            settings,
            request: None,
            ack: None,
        });
    }
    /// Save a local preference without blocking the UI or applying it before confirmation.
    pub fn persist_confirmed(&self, settings: crate::persistence::Settings) -> mpsc::Receiver<bool> {
        let (tx, rx) = mpsc::channel();
        if let Err(error) = self.storage_tx.send(StorageWrite {
            settings, request: None, ack: Some(tx),
        }) {
            if let Some(ack) = error.0.ack { let _ = ack.send(false); }
        }
        rx
    }
    pub fn flush(&self, settings: crate::persistence::Settings) -> bool {
        let (tx, rx) = std::sync::mpsc::channel();
        if self
            .storage_tx
            .send(StorageWrite {
                settings,
                request: None,
                ack: Some(tx),
            })
            .is_err()
        {
            return false;
        }
        rx.recv_timeout(Duration::from_secs(5)).unwrap_or(false)
    }
    pub fn connect(&mut self, server: &str, token: &str) -> anyhow::Result<()> {
        anyhow::ensure!(!self.is_connected(), crate::tr!("Bitte zuerst die bestehende Verbindung trennen."));
        let desktop_id = crate::persistence::desktop_client_id()?;
        let client = FactoryClient::new_desktop(server, token, desktop_id)?;
        if let Some(task) = self.connection.take() {
            task.abort();
        }
        self.generation += 1;
        self.client = Some(client.clone());
        self.desktop_id = Some(desktop_id);
        let sender = self.sender();
        self.connection = Some(self.runtime.spawn(async move {
            let mut cursor = None;
            let mut feed: Option<AbortTask> = None;
            let (dirty_tx, mut dirty_rx) = tokio::sync::watch::channel(0_u64);
            let mut timer = tokio::time::interval(Duration::from_secs(3));
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! { _ = timer.tick() => {}, _ = dirty_rx.changed() => { tokio::time::sleep(Duration::from_millis(120)).await; } }
                match client.desktop_snapshot_with_peer().await {
                    Ok((snapshot, peer)) => {
                        // Restart on server restore: durable cursors may move backwards.
                        if cursor.is_some_and(|v| snapshot.event_cursor < v) {
                            feed = None;
                        }
                        cursor = Some(snapshot.event_cursor);
                        if feed.as_ref().is_none_or(|t| t.0.is_finished()) {
                            let client = client.clone(); let dirty = dirty_tx.clone(); let mut after = snapshot.event_cursor;
                            feed = Some(AbortTask(tokio::spawn(async move {
                                loop {
                                    if let Ok(mut stream) = client.desktop_event_stream(after).await {
                                        while let Ok(Some(e)) = stream.next().await { after = e.sequence; if dirty.send(after).is_err() { return; } }
                                    }
                                    tokio::time::sleep(Duration::from_secs(2)).await;
                                }
                            })));
                        }
                        sender.send(Event::SnapshotPeer(peer));
                        sender.send(Event::Snapshot(Ok(snapshot)));
                        sender.send(Event::Access(client.management_request("GET", "/v1/me", &Value::Null).await.map_err(|e| e.to_string()).and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))));
                        sender.send(Event::Instances(client.instances().await.map_err(|e| e.to_string())));
                        sender.send(Event::Clients(client.management_request("GET","/v1/desktop/clients",&Value::Null).await.map_err(|e|e.to_string()).and_then(|v|serde_json::from_value(v).map_err(|e|e.to_string()))));
                        sender.send(Event::Notifications(client.notifications(None).await.map_err(|e| e.to_string()), false));
                    }
                    Err(error) => sender.send(Event::Snapshot(Err(snapshot_error(&error)))),
                }
            }
        }));
        Ok(())
    }
    pub fn is_connected(&self) -> bool {
        self.client.is_some()
    }
    pub fn disconnect(&mut self) {
        if let Some(task) = self.connection.take() {
            task.abort();
        }
        self.client = None;
        self.desktop_id = None;
        // Ignore already queued responses from the previous connection.
        self.generation += 1;
    }
    fn sender(&self) -> Sender {
        Sender {
            tx: self.tx.clone(),
            ctx: self.ctx.clone(),
            generation: self.generation,
        }
    }
    pub fn context(&self) -> &egui::Context {
        &self.ctx
    }
    #[cfg(test)]
    pub(crate) fn emit_for_test(&self, event: Event) {
        self.sender().send(event);
    }
    pub fn request(&self, id: u64, method: String, path: String, body: Value) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let sender = self.sender();
        self.runtime.spawn(async move {
            sender.send(Event::Request(
                id,
                perform_request(&client, &method, &path, &body).await,
            ));
        });
    }
    pub fn durable_request(
        &self,
        id: u64,
        settings: crate::persistence::Settings,
        method: String,
        path: String,
        body: Value,
    ) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let sender = self.sender();
        let _ = self.storage_tx.send(StorageWrite {
            settings,
            request: Some(DurableRequest {
                id,
                client,
                sender,
                method,
                path,
                body,
            }),
            ack: None,
        });
    }
    pub fn events(&self, after: u64) {
        let Some(c) = self.client.clone() else {
            return;
        };
        let s = self.sender();
        self.runtime.spawn(async move {
            s.send(Event::Events(
                c.management_request(
                    "GET",
                    &format!("/v1/desktop/events?after={after}"),
                    &Value::Null,
                )
                .await
                .map_err(|e| e.to_string())
                .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string())),
            ));
        });
    }
    pub fn task_payload(&self, id: TicketId, revision: Revision, request: u64) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let sender = self.sender();
        self.runtime.spawn(async move {
            sender.send(Event::TaskPayload(
                id,
                revision,
                request,
                client.task_payload(id).await.map_err(|e| e.to_string()),
            ));
        });
    }
    pub fn task_history(&self, dialog: RequestId, ticket: TicketId, before: Option<u64>) {
        let Some(client) = self.client.clone() else { return };
        let sender = self.sender();
        self.runtime.spawn(async move {
            let mut path = format!("/v1/tasks/{ticket}/history");
            if let Some(before) = before {
                path.push_str(&format!("?before={before}"));
            }
            let result = client.management_request("GET", &path, &Value::Null).await
                .map_err(|e| e.to_string())
                .and_then(|value| serde_json::from_value(value).map_err(|e| e.to_string()));
            sender.send(Event::TaskHistory(dialog, before, result));
        });
    }
    pub fn history(&self, before: Option<u64>) {
        let Some(c) = self.client.clone() else {
            return;
        };
        let s = self.sender();
        self.runtime.spawn(async move {
            let path = before.map_or_else(
                || "/v1/events/history".to_string(),
                |v| format!("/v1/events/history?before={v}"),
            );
            s.send(Event::History(
                c.management_request("GET", &path, &Value::Null)
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string())),
            ));
        });
    }
    pub fn notifications(&self, before: u64) {
        let Some(c) = self.client.clone() else {
            return;
        };
        let s = self.sender();
        self.runtime.spawn(async move {
            s.send(Event::Notifications(
                c.notifications(Some(before))
                    .await
                    .map_err(|e| e.to_string()),
                true,
            ));
        });
    }
    pub fn notification_states(&self, ids: Vec<u64>) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let sender = self.sender();
        self.runtime.spawn(async move {
            let result = async {
                let mut states = Vec::new();
                for ids in ids.chunks(1000) {
                    let value = client
                        .management_request(
                            "POST",
                            "/v1/notifications/status",
                            &serde_json::json!({"ids":ids}),
                        )
                        .await
                        .map_err(|e| e.to_string())?;
                    states.extend(
                        serde_json::from_value::<Vec<NotificationReadState>>(value)
                            .map_err(|e| e.to_string())?,
                    );
                }
                Ok(states)
            }
            .await;
            sender.send(Event::NotificationStates(result));
        });
    }
    pub fn archive(&self, id: CommandId, mode: ArchiveRead, request: u64, versions: bool) {
        let Some(c) = self.client.clone() else {
            return;
        };
        let s = self.sender();
        self.runtime.spawn(async move {
            let (after, before) = match mode {
                ArchiveRead::Recent => (None, None),
                ArchiveRead::Newer(v) => (Some(v), None),
                ArchiveRead::Older(v) => (None, Some(v)),
            };
            s.send(Event::Archive(
                id,
                request,
                mode,
                if versions {
                    c.archive_window(id, after, before).await
                } else {
                    c.conversation_window(id, after, before).await
                }
                .map(Box::new)
                .map_err(|e| e.to_string()),
            ));
        });
    }
    pub fn pick_attachments(&self, target: RequestId) {
        let s = self.sender();
        self.runtime.spawn_blocking(move || {
            if let Some(paths) = rfd::FileDialog::new()
                .set_title(crate::tr!("Bilder und Dateien anhängen"))
                .pick_files()
            {
                for path in paths {
                    s.send(Event::Attachment(target, load_attachment(&path)));
                }
            }
        });
    }
    pub fn load_attachments(&self, target: RequestId, paths: Vec<PathBuf>) {
        let s = self.sender();
        self.runtime.spawn_blocking(move || {
            for path in paths {
                s.send(Event::Attachment(target, load_attachment(&path)));
            }
        });
    }
    pub fn attach_clipboard_image(&self, target: RequestId, name: &'static str, bytes: Vec<u8>) {
        let s = self.sender();
        self.runtime.spawn_blocking(move || {
            s.send(Event::Attachment(target, clipboard_attachment(name, &bytes)));
        });
    }
    pub fn save_json(&self, name: &str, value: Value) {
        let s = self.sender();
        let name = name.to_string();
        self.runtime.spawn_blocking(move || {
            if let Some(path) = rfd::FileDialog::new().set_file_name(name).save_file() {
                s.send(Event::Saved(
                    crate::persistence::write_new(
                        &path,
                        &serde_json::to_vec_pretty(&value).unwrap_or_default(),
                    )
                    .map(|()| path.display().to_string())
                    .map_err(|e| e.to_string()),
                ));
            }
        });
    }
    pub fn save_screenshot(&self, path: PathBuf, pixels: std::sync::Arc<egui::ColorImage>) {
        let s = self.sender();
        self.runtime.spawn_blocking(move || {
            let result = (|| -> anyhow::Result<String> {
                let bytes = pixels
                    .pixels
                    .iter()
                    .flat_map(|p| p.to_array())
                    .collect::<Vec<_>>();
                let image =
                    image::RgbaImage::from_raw(pixels.size[0] as u32, pixels.size[1] as u32, bytes)
                        .ok_or_else(|| anyhow::anyhow!("{}", crate::tr!("Ungültiger Screenshot", )))?;
                let mut data = std::io::Cursor::new(Vec::new());
                image::DynamicImage::ImageRgba8(image)
                    .write_to(&mut data, image::ImageFormat::Png)?;
                crate::persistence::write_new(&path, &data.into_inner())?;
                Ok(path.display().to_string())
            })();
            s.send(Event::Saved(result.map_err(|e| e.to_string())));
        });
    }
    pub fn preview(
        &self,
        key: String,
        image: PromptImage,
        full: bool,
        max_texture_side: usize,
    ) {
        let s = self.sender();
        self.runtime.spawn_blocking(move || {
            let bytes = crate::persistence::image_bytes(&image);
            let result = decode_preview(bytes, full, max_texture_side);
            s.send(Event::Preview(key, result.map_err(|e| e.to_string())));
        });
    }
    pub fn save_archive_file(&self, command: CommandId, hash: String, name: String) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let s = self.sender();
        self.runtime.spawn(async move {
            let result = crate::persistence::download_file(&client, command, &hash, &name).await;
            s.send(Event::Saved(result.map_err(|e| e.to_string())));
        });
    }
    pub fn export_archive(&self, commands: Vec<CommandId>) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let s = self.sender();
        self.runtime.spawn(async move {
            let result = async {
                use std::io::Write;
                let path = tokio::task::spawn_blocking(|| rfd::FileDialog::new().set_file_name("opencode-archiv.jsonl").save_file()).await?;
                let Some(path) = path else { return Ok::<_, anyhow::Error>(crate::tr!("Export abgebrochen").to_string()); };
                anyhow::ensure!(!path.exists(), crate::tr!("Zieldatei existiert bereits"));
                let mut file = tempfile::NamedTempFile::new_in(path.parent().unwrap_or(std::path::Path::new(".")))?;
                for command in commands {
                    let mut page = client.archive(command, 0, None).await?;
                    let revision = page.status.as_ref().map_or(0, |s| s.revision);
                    writeln!(file, "{}", serde_json::json!({"command_id":command,"status":page.status,"termination":page.termination}))?;
                    loop {
                        for entry in &page.entries { writeln!(file, "{}", serde_json::json!({"command_id":command,"entry":entry}))?; }
                        let after = page.next_after;
                        if page.entries.is_empty() { break; }
                        page = client.archive(command, after, Some(revision)).await?;
                        anyhow::ensure!(page.next_after >= after, crate::tr!("Archivcursor rückläufig"));
                    }
                }
                writeln!(file, "{{\"export_complete\":true}}")?;
                file.as_file().sync_all()?; file.persist_noclobber(&path).map_err(|e| e.error)?;
                Ok(path.display().to_string())
            }.await;
            s.send(Event::Saved(result.map_err(|e| e.to_string())));
        });
    }
    pub fn save_input_file(&self, name: String, data: String) {
        let s = self.sender();
        self.runtime.spawn_blocking(move || {
            use base64::Engine;
            let result = (|| -> anyhow::Result<String> {
                anyhow::ensure!(data.len() <= 6 * 1024 * 1024, crate::tr!("Inline-Datei zu groß"));
                let bytes = base64::engine::general_purpose::STANDARD.decode(data)?;
                if let Some(path) = rfd::FileDialog::new()
                    .set_file_name(
                        std::path::Path::new(&name)
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy(),
                    )
                    .save_file()
                {
                    crate::persistence::write_new(&path, &bytes)?;
                    Ok(path.display().to_string())
                } else {
                    Ok(crate::tr!("Export abgebrochen").into())
                }
            })();
            s.send(Event::Saved(result.map_err(|e| e.to_string())));
        });
    }
    pub fn start_local(&self) {
        let s = self.sender();
        self.runtime.spawn(async move {
            s.send(Event::Local(
                crate::persistence::start_local()
                    .await
                    .map_err(|e| e.to_string()),
            ));
        });
    }
    pub fn local_runners(&self, server: String, action: crate::local_runners::Action) {
        let Some(client) = self.client.clone() else { return; };
        let mut sender = self.sender();
        // Local process operations must finish even if the UI changes connections.
        sender.generation = u64::MAX;
        self.runtime.spawn(async move {
            sender.send(Event::LocalRunners(crate::local_runners::perform(&client, &server, action).await));
        });
    }
}

fn decode_preview(
    bytes: Vec<u8>,
    full: bool,
    max_texture_side: usize,
) -> anyhow::Result<egui::ColorImage> {
    let mut reader =
        image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(64 * 1024 * 1024);
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    reader.limits(limits);
    let decoded = reader.decode()?;
    let max_texture_side = u32::try_from(max_texture_side.max(1)).unwrap_or(u32::MAX);
    let (max_width, max_height) = if full {
        (max_texture_side, max_texture_side)
    } else {
        (256.min(max_texture_side), 192.min(max_texture_side))
    };
    let decoded = decoded.thumbnail(max_width, max_height).into_rgba8();
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        [decoded.width() as usize, decoded.height() as usize],
        decoded.as_raw(),
    ))
}
fn snapshot_error(error: &ClientError) -> String {
    match error {
        ClientError::Http { status, .. } if status.as_u16() == 404 =>
            crate::tr!("Dieser Server unterstützt die Desktop-API noch nicht (/v1/desktop/snapshot fehlt). Wahrscheinlich läuft eine ältere factoryd-Version oder die Serveradresse ist falsch. Ein fehlendes Projekt ist nicht die Ursache. Server aktualisieren und neu starten oder über „Verbindung …“ eine separate lokale Factory starten. Neu bauen allein aktualisiert laufende Prozesse nicht.").into(),
        ClientError::Http { status, .. } if matches!(status.as_u16(), 401 | 403) =>
            crate::tr!("Zugang zum Server abgelehnt. Unter „Verbindung …“ Serveradresse und Zugangstoken prüfen; ein Projekt muss dafür noch nicht eingerichtet sein.").into(),
        ClientError::Transport =>
            crate::tr!("Server nicht erreichbar. Unter „Verbindung …“ einen vorhandenen Server wählen oder eine lokale Factory starten. Die Verbindung wird automatisch erneut geprüft.").into(),
        _ => error.to_string(),
    }
}

#[cfg(test)]
mod startup_tests {
    use super::*;
    #[tokio::test]
    async fn desktop_reconnects_and_client_clones_send_the_same_presence_identity() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for _ in 0..3 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = Vec::new();
                loop {
                    let mut chunk = [0; 4096];
                    let read = stream.read(&mut chunk).unwrap();
                    request.extend_from_slice(&chunk[..read]);
                    if read == 0 || request.windows(4).any(|bytes| bytes == b"\r\n\r\n") { break; }
                }
                requests.push(String::from_utf8(request).unwrap().to_ascii_lowercase());
                stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
            }
            requests
        });
        let id = RequestId::new();
        let endpoint = format!("http://{address}");
        let client = FactoryClient::new_desktop(
            &endpoint, "desktop-presence-test-token-at-least-32-bytes", id,
        ).unwrap();
        let _ = client.desktop_snapshot().await;
        let _ = client.clone().desktop_snapshot().await;
        let reconnected = FactoryClient::new_desktop(
            &endpoint, "desktop-presence-rotated-token-at-least-32-bytes", id,
        ).unwrap();
        let _ = reconnected.desktop_snapshot().await;
        for request in server.join().unwrap() {
            assert!(request.contains(&format!("x-factory-client-id: {id}\r\n")));
            assert!(request.contains("x-factory-client-kind: desktop\r\n"));
        }
    }

    #[tokio::test]
    async fn missing_desktop_endpoint_explains_old_server_not_missing_project() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 4096];
            let _ = stream.read(&mut request).unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
        });
        let client = FactoryClient::new(
            &format!("http://{address}"),
            "desktop-startup-test-token-at-least-32-bytes",
        )
        .unwrap();
        let error = client.desktop_snapshot().await.unwrap_err();
        let message = snapshot_error(&error);
        assert!(message.contains("/v1/desktop/snapshot"));
        assert!(message.contains("Ein fehlendes Projekt ist nicht die Ursache"));
        assert!(message.contains("Neu bauen allein"));
        assert!(!message.contains("aktuelle Revision"));
        server.join().unwrap();
    }
}
impl Drop for Backend {
    fn drop(&mut self) {
        if let Some(task) = self.connection.take() {
            task.abort();
        }
    }
}
#[derive(Clone)]
struct Sender {
    tx: mpsc::Sender<Envelope>,
    ctx: egui::Context,
    generation: u64,
}
impl Sender {
    fn send(&self, event: Event) {
        let _ = self.tx.send(Envelope {
            generation: self.generation,
            event,
        });
        self.ctx.request_repaint();
    }
}
struct StorageWrite {
    settings: crate::persistence::Settings,
    request: Option<DurableRequest>,
    ack: Option<std::sync::mpsc::Sender<bool>>,
}
struct DurableRequest {
    id: u64,
    client: FactoryClient,
    sender: Sender,
    method: String,
    path: String,
    body: Value,
}
struct AbortTask(tokio::task::JoinHandle<()>);
async fn perform_request(
    client: &FactoryClient,
    method: &str,
    path: &str,
    body: &Value,
) -> Result<Value, Failure> {
    if matches!(
        method,
        "LOCAL_SSH_READ" | "LOCAL_SSH_SAVE" | "RUNNER_HELPER"
    ) {
        crate::operations::execute(method, path, body).await
    } else {
        client
            .management_request_detailed(method, path, body)
            .await
            .map_err(|error| {
                if path.starts_with("/v1/model-settings/accesses/") && path.contains("/oauth") {
                    let message = error.message.unwrap_or_else(|| crate::tr!("Die Anfrage zur ChatGPT-Anmeldung konnte nicht abgeschlossen werden.").into());
                    let mut failure = Failure::from(error.error);
                    failure.message = message;
                    return failure;
                }
                let missing=matches!(&error.error,ClientError::Http {status,..} if status.as_u16()==404);
                let mut failure=Failure::from(error);
                if missing && path.starts_with("/v1/tasks/") && path.ends_with("/edit") {
                    failure.message=crate::tr!("Bearbeitungsreservierung nicht verfügbar. Aktuellen Serverstand prüfen; bei einem älteren Server factoryd aktualisieren und neu starten (Schema 33). Ohne bestätigte Reservierung wird die Aufgabe nicht zum Bearbeiten geöffnet.").into();
                }
                failure
            })
    }
}
struct RuntimeGuard(Option<tokio::runtime::Runtime>);
impl std::ops::Deref for RuntimeGuard {
    type Target = tokio::runtime::Runtime;
    fn deref(&self) -> &Self::Target {
        self.0.as_ref().expect("runtime alive")
    }
}
impl Drop for RuntimeGuard {
    fn drop(&mut self) {
        if let Some(runtime) = self.0.take() {
            runtime.shutdown_background();
        }
    }
}
impl Drop for AbortTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn clipboard_attachment(name: &str, bytes: &[u8]) -> Result<InputAttachment, String> {
    let image = PromptImage::from_bytes(name.into(), bytes).map_err(str::to_string)?;
    let (width, height) = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .and_then(|reader| reader.into_dimensions().map_err(std::io::Error::other))
        .map_err(|_| crate::tr!("Das Bild in der Zwischenablage konnte nicht gelesen werden.").to_owned())?;
    if width == 0 || height == 0 || width > 8192 || height > 8192 {
        return Err(crate::tr!("Bildgröße ungültig (maximal 8192 × 8192).").into());
    }
    Ok(InputAttachment::Image(image))
}

#[cfg(test)]
#[test]
fn clipboard_attachment_keeps_the_screenshot_bytes() {
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image::RgbaImage::new(2, 2))
        .write_to(&mut png, image::ImageFormat::Png).unwrap();
    let bytes = png.into_inner();
    let InputAttachment::Image(result) = clipboard_attachment(crate::tr!("Zwischenablage.png"), &bytes).unwrap()
        else { panic!("Expected an image attachment") };
    assert_eq!(result, PromptImage::from_bytes(crate::tr!("Zwischenablage.png").into(), &bytes).unwrap());
    assert!(clipboard_attachment(crate::tr!("Zwischenablage.png"), &bytes[..8]).is_err());
}

#[cfg(test)]
#[test]
fn full_preview_respects_the_renderer_texture_limit() {
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image::RgbaImage::new(40, 21))
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();

    let preview = decode_preview(png.into_inner(), true, 20).unwrap();

    assert_eq!(preview.width(), 20);
    assert!(preview.height() <= 20);
}

fn load_attachment(path: &std::path::Path) -> Result<InputAttachment, String> {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or(crate::tr!("Ungültiger Dateiname"))?
        .to_string();
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(
        &mut std::io::Read::take(&mut file, (TASK_FILE_BYTES + 1) as u64),
        &mut bytes,
    )
    .map_err(|e| e.to_string())?;
    if let Ok(image) = PromptImage::from_bytes(name.clone(), &bytes) {
        Ok(InputAttachment::Image(image))
    } else {
        PromptFile::from_bytes(name, &bytes)
            .map(InputAttachment::File)
            .map_err(str::to_string)
    }
}
