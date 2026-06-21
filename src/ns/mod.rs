// namespace registry: build a layer's namespace once, persist it to disk,
// enter it later with no daemon round trip

mod build;
mod pin;
mod reg;

pub use reg::Registry;
