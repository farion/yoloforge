use super::*;

impl Store {
    /// Count each native session/message once, including across continuations.
    /// Newer command snapshots take precedence over late historical backfill.
    pub fn project_usage(&self, project: ProjectId) -> Result<ProjectUsage> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction()?;
        usage_in(&tx, project)
    }
}

pub(super) fn usage_in(tx: &Transaction<'_>, project: ProjectId) -> Result<ProjectUsage> {
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM active_projects WHERE id=?1)",
        [project.to_string()],
        |r| r.get(0),
    )?;
    if !exists {
        return Err(StoreError::NotFound);
    }
    let mut stmt = tx.prepare("WITH samples AS (
            SELECT COALESCE(NULLIF(json_extract(c.data,'$.agent_spec.role'),''),'work') AS role,
                json_extract(m.data,'$.model.providerID') AS provider,
                json_extract(m.data,'$.model.id') AS model,
                json_extract(m.data,'$.tokens') AS tokens,
                json_extract(m.data,'$.cost') AS cost,
                json_extract(m.data,'$.time.completed') IS NOT NULL AS completed,
                json_extract(m.data,'$.type') AS message_type,
                ROW_NUMBER() OVER (PARTITION BY COALESCE(json_extract(c.data,'$.agent_spec.continuation.root_command_id'),c.id),m.message_id ORDER BY c.rowid DESC,m.sequence DESC) AS rank
            FROM archive_messages m JOIN commands c ON c.id=m.command_id JOIN streams s ON s.id=c.stream_id
            WHERE s.project_id=?1)
            SELECT role,provider,model,tokens,cost,completed FROM samples WHERE rank=1 AND message_type='assistant'")?;
    let mut rows = stmt.query([project.to_string()])?;
    let mut groups = std::collections::BTreeMap::new();
    while let Some(row) = rows.next()? {
        let key = (
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1).ok().flatten(),
            row.get::<_, Option<String>>(2).ok().flatten(),
        );
        let group = groups.entry(key.clone()).or_insert_with(|| UsageGroup {
            role: key.0,
            provider_id: key.1,
            model_id: key.2,
            ..Default::default()
        });
        group.assistant_messages += 1;
        if !row.get::<_, bool>(5)? {
            group.unfinished_messages += 1;
        }
        let raw = row.get::<_, Option<String>>(3).ok().flatten();
        if let Some(tokens) = raw.and_then(|v| serde_json::from_str::<TokenUsage>(&v).ok()) {
            fn add(a: &mut u64, b: u64) -> Result<()> {
                *a = a
                    .checked_add(b)
                    .ok_or(StoreError::Invalid("Usage token total overflow"))?;
                Ok(())
            }
            add(&mut group.tokens.input, tokens.input)?;
            add(&mut group.tokens.output, tokens.output)?;
            add(&mut group.tokens.reasoning, tokens.reasoning)?;
            add(&mut group.tokens.cache.read, tokens.cache.read)?;
            add(&mut group.tokens.cache.write, tokens.cache.write)?;
        } else {
            group.messages_missing_tokens += 1;
        }
        // Archive content is untrusted. Non-numeric/negative values are unknown.
        let cost = row
            .get::<_, Option<f64>>(4)
            .ok()
            .flatten()
            .filter(|v| v.is_finite() && *v >= 0.0);
        if let Some(cost) = cost {
            group.reported_cost_usd += cost;
            if !group.reported_cost_usd.is_finite() {
                return Err(StoreError::Invalid("Usage cost total overflow"));
            }
        } else {
            group.messages_missing_cost += 1;
        }
    }
    Ok(ProjectUsage { token_budget:Some(super::token_budgets::load(tx,project)?),project_id:project,groups:groups.into_values().collect(),
            source:"Latest archived native messages; incomplete coverage possible; reported cost is not an invoice; reasoning/cache counters are separate and must not be summed blindly".into() })
}

/// Continuation archives can contain the entire native conversation. Deduplicate
/// each message across executions and attribute task usage by its native prompt,
/// not by whichever execution last archived the message.
pub(super) fn observed_token_totals(
    tx: &Transaction<'_>,
    commands: &[RunnerCommand],
) -> Result<ObservedTokenTotals> {
    use std::collections::BTreeMap;

    let prompts: BTreeMap<_, _> = commands
        .iter()
        .filter(|c| c.kind == CommandKind::RunAgent)
        .map(|c| (agent_message_id(c.id), (c.session_root(), c.id)))
        .collect();
    let mut stmt = tx.prepare(
        "WITH latest AS (
            SELECT MAX(sequence) AS sequence FROM archive_messages GROUP BY command_id,message_id
         ), messages AS (
            SELECT COALESCE(json_extract(c.data,'$.agent_spec.continuation.root_command_id'),c.id) AS root,
                m.message_id,m.sequence,c.rowid AS command_order,
                json_extract(m.data,'$.type') AS kind,
                CASE WHEN json_type(m.data,'$.time.created')='integer'
                     THEN json_extract(m.data,'$.time.created') END AS created,
                json_extract(m.data,'$.tokens') AS tokens
            FROM latest JOIN archive_messages m USING(sequence) JOIN commands c ON c.id=m.command_id
         ), samples AS (
            SELECT *, ROW_NUMBER() OVER (PARTITION BY root,message_id ORDER BY command_order DESC,sequence DESC) AS rank
            FROM messages WHERE kind IN ('user','assistant')
         )
         SELECT root,message_id,kind,created,tokens FROM samples WHERE rank=1
         ORDER BY root,created,(kind!='user'),sequence",
    )?;
    let mut rows = stmt.query([])?;
    let mut totals = ObservedTokenTotals::default();
    let mut session = None;
    let mut execution = None;
    while let Some(row) = rows.next()? {
        let Ok(root) = row.get::<_, String>(0)?.parse::<CommandId>() else {
            continue;
        };
        if session != Some(root) {
            session = Some(root);
            execution = None;
        }
        let kind = row.get::<_, String>(2)?;
        let has_time = row.get::<_, Option<i64>>(3)?.is_some_and(|t| t >= 0);
        if kind == "user" {
            let id = row.get::<_, String>(1)?;
            execution = prompts
                .get(&id)
                .filter(|(session, _)| has_time && *session == root)
                .map(|(_, command)| *command);
            continue;
        }
        let Some(tokens) = row
            .get::<_, Option<String>>(4)
            .ok()
            .flatten()
            .and_then(|raw| serde_json::from_str::<TokenUsage>(&raw).ok())
            .and_then(|tokens| tokens.input.checked_add(tokens.output))
        else {
            continue;
        };
        let total = totals.by_session.entry(root).or_default();
        *total = total.saturating_add(tokens);
        if let Some(id) = execution.filter(|_| has_time) {
            let total = totals.by_execution.entry(id).or_default();
            *total = total.saturating_add(tokens);
        }
    }
    Ok(totals)
}
