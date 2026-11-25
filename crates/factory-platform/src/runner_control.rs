//! Loopback-only, per-runner control. Stopping never signals an unverified/recycled PID.
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use std::{
    io::{BufRead, BufReader, Write},
    net::{Ipv4Addr, SocketAddrV4, TcpStream},
    path::PathBuf,
    time::Duration,
};

#[derive(Serialize, Deserialize)]
pub struct Endpoint {
    pub port: u16,
    pub secret: String,
    pub pid: u32,
    pub runner: String,
    pub server: String,
    pub journal: PathBuf,
}

impl Endpoint {
    pub fn request(&self, action: &str) -> Result<()> {
        ensure!(
            matches!(action, "ping" | "stop"),
            "Unknown runner control action"
        );
        ensure!(
            self.secret.len() == 72
                && self
                    .secret
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() || b == b'-'),
            "Invalid runner control credential"
        );
        let mut stream = TcpStream::connect_timeout(
            &SocketAddrV4::new(Ipv4Addr::LOCALHOST, self.port).into(),
            Duration::from_secs(1),
        )?;
        stream.set_read_timeout(Some(Duration::from_secs(1)))?;
        stream.set_write_timeout(Some(Duration::from_secs(1)))?;
        writeln!(stream, "{} {action}", self.secret)?;
        let mut response = String::new();
        std::io::Read::take(BufReader::new(stream), 128).read_line(&mut response)?;
        ensure!(
            response == format!("OK {}\n", self.pid),
            "Runner control identity not confirmed"
        );
        Ok(())
    }
}
