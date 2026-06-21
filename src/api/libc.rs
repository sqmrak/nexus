/// libc identity carried as data, not an enum, so a new libc adds no code
/// branch. the core reads the loader path and the layer's namespace resolves it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Libc {
    /// free form: "glibc", "musl", "uclibc", "static"
    pub name: String,
    /// loader inside the layer, or none for a static layer
    pub loader: Option<String>,
}

impl Libc {
    pub fn is_static(&self) -> bool {
        self.loader.is_none()
    }
}
