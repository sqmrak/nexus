/// a blake3 tree hash naming a set of objects in the content store
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ObjectHash(String);

impl ObjectHash {
    pub fn new(hash: impl Into<String>) -> Self {
        ObjectHash(hash.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ObjectHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// a numbered system generation. monotonic, never reused; rollback swaps
/// which one is current
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Gen(u64);

impl Gen {
    pub fn new(n: u64) -> Self {
        Gen(n)
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for Gen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
