// content-addressed store: objects by hash, gens for rollback
// overlay is the reference backend; erofs slots in behind StoreBackend

mod cas;
mod erofs;
mod generations;
mod overlay;

pub use cas::Store;
pub use erofs::ErofsBackend;
pub use generations::Gens;
pub use overlay::OverlayBackend;
