//! Run IDs: UUIDv7, generated on the client so `init()` never waits for the server.
//! They sort by creation time, which keeps database indexes append-friendly.

use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RunId(Uuid);

impl RunId {
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let id = Uuid::from_slice(bytes).ok()?;
        (id.get_version_num() == 7).then_some(Self(id))
    }

    pub fn as_bytes(&self) -> &[u8; 16] {
        self.0.as_bytes()
    }

    /// Short lowercase hex form used in URLs and logs.
    pub fn to_simple(&self) -> String {
        self.0.simple().to_string()
    }
}

impl Default for RunId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for RunId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.simple().fmt(f)
    }
}

impl std::str::FromStr for RunId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(s).map(Self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_time_ordered() {
        let a = RunId::new();
        let b = RunId::new();
        assert!(a < b);
    }

    #[test]
    fn bytes_round_trip() {
        let id = RunId::new();
        assert_eq!(RunId::from_bytes(id.as_bytes()), Some(id));
        assert_eq!(RunId::from_bytes(&[0; 3]), None);
    }

    #[test]
    fn rejects_non_v7() {
        assert_eq!(RunId::from_bytes(Uuid::nil().as_bytes()), None);
    }

    #[test]
    fn string_round_trip() {
        let id = RunId::new();
        assert_eq!(id.to_string().parse::<RunId>().unwrap(), id);
    }
}
