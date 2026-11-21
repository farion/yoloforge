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
    pub fn create_enrollment(&self, body: CreateEnrollment) -> Result<Enrollment> {
        if !(30..=900).contains(&body.ttl_seconds)
            || (body.role == DeviceRole::Runner) != body.runner_id.is_some()
        {
            return Err(StoreError::Invalid(
                "Enrollment role, runner or expiry invalid",
            ));
        }
        let code = secret();
        let now = super::super::unix_ms();
        let expires_unix_ms = now + u64::from(body.ttl_seconds) * 1000;
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM enrollments WHERE expires_unix_ms<=?1", [now])?;
        tx.execute(
            "INSERT INTO enrollments(code_hash,expires_unix_ms,data) VALUES(?1,?2,?3)",
            params![
                digest(&code),
                expires_unix_ms,
                serde_json::to_string(&body)?
            ],
        )?;
        tx.commit()?;
        Ok(Enrollment {
            code,
            expires_unix_ms,
        })
    }
    pub fn redeem_enrollment(&self, body: RedeemEnrollment) -> Result<DeviceCredential> {
        line(&body.name, 80)?;
        if body.code.len() > 100 {
            return Err(StoreError::NotFound);
        }
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let hash = digest(&body.code);
        let now = super::super::unix_ms();
        let data: Option<String> = tx
            .query_row(
                "SELECT data FROM enrollments WHERE code_hash=?1 AND expires_unix_ms>?2",
                params![hash, now],
                |r| r.get(0),
            )
            .optional()?;
        let grant: CreateEnrollment = serde_json::from_str(&data.ok_or(StoreError::NotFound)?)?;
        let device = Device {
            id: DeviceId::new(),
            name: body.name,
            role: grant.role,
            runner_id: grant.runner_id,
            created_unix_ms: now,
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
        tx.execute("DELETE FROM enrollments WHERE code_hash=?1", [hash])?;
        event(&tx, "device.enrolled", device.id.to_string(), &device)?;
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
