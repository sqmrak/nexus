/// typed flags the core acts on; fork-defined flags ride in extra so the
/// core stays ignorant of policy it does not own
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LayerFlags {
    /// layer is itself a meta-distro (an imported bedrock stratum)
    pub meta: bool,
    /// non-standard layout (nixos, guix, gobolinux): resolver indexes real
    /// paths, the core does not assume fhs
    pub non_fhs: bool,
    /// immutable, atomically updated (silverblue, microos): no persistent
    /// writable upper, changes land in a new gen
    pub atomic: bool,
    /// lives until reboot: writable upper in tmpfs
    pub ephemeral: bool,
    /// version frozen, forge does not update it. policy, core ignores
    pub pinned: bool,
    /// not shown in launcher or PATH. policy, core ignores
    pub hidden: bool,
    /// fork-defined flags the core does not know. carried verbatim
    pub extra: Vec<String>,
}

impl LayerFlags {
    /// true when ephemeral or atomic: in both cases there is no persistent
    /// on-disk upper, so the store skips the writable overlay
    pub fn no_persistent_upper(&self) -> bool {
        self.ephemeral || self.atomic
    }

    /// the layer wants a writable upper in tmpfs, lost on reboot. atomic wins
    /// if both are set: atomic takes no upper at all
    pub fn ephemeral_upper(&self) -> bool {
        self.ephemeral && !self.atomic
    }
}
