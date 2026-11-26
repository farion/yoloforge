use super::*;
fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
fn secret() -> String {
    format!(
        "yf_{}{}",
        RequestId::new().to_string().replace('-', ""),
        RequestId::new().to_string().replace('-', "")
    )
}
impl Store {
    pub fn authenticate_device(&self, token: &str) -> Result<Option<Device>> {
        let db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let value: Option<String> = db
            .query_row(
                "SELECT data FROM devices WHERE token_hash=?1 AND revoked=0",
                [digest(token)],
                |r| r.get(0),
            )
            .optional()?;
        value
            .map(|s| serde_json::from_str(&s).map_err(Into::into))
            .transpose()
    }
    pub fn create_device(&self, body: CreateDevice) -> Result<DeviceCredential> {
        line(&body.name, 80)?;
        if (body.role == DeviceRole::Runner) != body.runner_id.is_some() {
            return Err(StoreError::Invalid("Invalid device role or runner binding"));
        }
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let device = Device {
            id: DeviceId::new(),
            name: body.name,
            role: body.role,
            runner_id: body.runner_id,
            created_unix_ms: super::super::unix_ms(),
            revoked: false,
        };
        let token = secret();
        tx.execute(
            "INSERT INTO devices(id,token_hash,data) VALUES(?1,?2,?3)",
            params![
                device.id.to_string(),
                digest(&token),
                serde_json::to_string(&device)?
            ],
        )?;
        event(&tx, "device.created", device.id.to_string(), &device)?;
        tx.commit()?;
        Ok(DeviceCredential { device, token })
    }
    pub fn devices(&self) -> Result<Vec<Device>> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction()?;
        records(&tx, "SELECT data FROM devices ORDER BY rowid")
    }
    pub fn revoke_device(&self, id: DeviceId) -> Result<Device> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let data: Option<String> = tx
            .query_row(
                "SELECT data FROM devices WHERE id=?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        let mut device: Device = serde_json::from_str(&data.ok_or(StoreError::NotFound)?)?;
        if !device.revoked {
            device.revoked = true;
            tx.execute(
                "UPDATE devices SET revoked=1,data=?1 WHERE id=?2",
                params![serde_json::to_string(&device)?, id.to_string()],
            )?;
            event(&tx, "device.revoked", id.to_string(), &device)?;
        }
        tx.commit()?;
        Ok(device)
    }
}
