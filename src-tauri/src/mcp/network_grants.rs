//! Trusted local exceptions: an exact literal address and port, never a LAN wildcard.
use super::policy::{self, Principal};
use rusqlite::params;
use std::{collections::BTreeSet, net::SocketAddr};
fn db() -> Result<rusqlite::Connection, String> {
    let c = policy::db()?;
    c.execute_batch("CREATE TABLE IF NOT EXISTS network_grants(principal TEXT NOT NULL,endpoint TEXT NOT NULL,PRIMARY KEY(principal,endpoint));").map_err(|_|"NETWORK_GRANT_STORE_UNAVAILABLE")?;
    Ok(c)
}
fn endpoint(raw: &str) -> Result<SocketAddr, String> {
    let socket: SocketAddr = raw
        .parse()
        .map_err(|_| "LITERAL_ADDRESS_AND_PORT_REQUIRED")?;
    if socket.port() == 0 || socket.ip().is_unspecified() || socket.ip().is_multicast() {
        return Err("INVALID_NETWORK_ENDPOINT".into());
    }
    Ok(socket)
}
/// Local UI only. Replaces this client's finite exception set atomically.
pub fn set(principal: &str, endpoints: &[String]) -> Result<(), String> {
    if endpoints.len() > 16 {
        return Err("NETWORK_GRANT_LIMIT".into());
    }
    let values: BTreeSet<_> = endpoints
        .iter()
        .map(|s| endpoint(s).map(|s| s.to_string()))
        .collect::<Result<_, _>>()?;
    let mut c = db()?;
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|_| "NETWORK_GRANT_STORE_UNAVAILABLE")?;
    let scopes: String = tx
        .query_row(
            "SELECT scopes FROM clients WHERE id=?1 AND revoked=0",
            [principal],
            |r| r.get(0),
        )
        .map_err(|_| "PRINCIPAL_INACTIVE")?;
    let scopes: Vec<String> = serde_json::from_str(&scopes).map_err(|_| "INVALID_SCOPES")?;
    if !scopes.iter().any(|s| s == "local_network") {
        return Err("LOCAL_NETWORK_NOT_GRANTED".into());
    }
    tx.execute("DELETE FROM network_grants WHERE principal=?1", [principal])
        .map_err(|_| "NETWORK_GRANT_STORE_UNAVAILABLE")?;
    for value in values {
        tx.execute(
            "INSERT INTO network_grants(principal,endpoint) VALUES(?1,?2)",
            params![principal, value],
        )
        .map_err(|_| "NETWORK_GRANT_STORE_UNAVAILABLE")?;
    }
    tx.commit()
        .map_err(|_| "NETWORK_GRANT_STORE_UNAVAILABLE".into())
}
pub fn allowed(p: &Principal) -> Result<BTreeSet<SocketAddr>, String> {
    policy::active(p)?;
    if !p.scopes.iter().any(|s| s == "local_network") {
        return Ok(BTreeSet::new());
    }
    let c = db()?;
    let mut q = c
        .prepare("SELECT endpoint FROM network_grants WHERE principal=?1 ORDER BY endpoint")
        .map_err(|_| "NETWORK_GRANT_STORE_UNAVAILABLE")?;
    let rows = q
        .query_map([&p.id], |r| r.get::<_, String>(0))
        .map_err(|_| "NETWORK_GRANT_STORE_UNAVAILABLE")?;
    rows.map(|r| {
        r.map_err(|_| "NETWORK_GRANT_STORE_UNAVAILABLE".to_owned())
            .and_then(|s| endpoint(&s))
    })
    .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exceptions_require_exact_endpoints() {
        assert!(endpoint("127.0.0.1:8080").is_ok());
        assert!(endpoint("[::1]:8080").is_ok());
        for bad in [
            "localhost:8080",
            "192.168.0.0/16",
            "0.0.0.0:80",
            "[::]:443",
            "127.0.0.1:0",
            "224.0.0.1:80",
        ] {
            assert!(endpoint(bad).is_err(), "{bad}");
        }
    }
}
