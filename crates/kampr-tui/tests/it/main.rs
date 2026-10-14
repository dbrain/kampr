#[global_allocator]
static ALLOCATOR: kampr_testkit::alloc::Counting = kampr_testkit::alloc::Counting;

mod desk;
mod images;
mod keys;
mod links;
mod manage;
mod pointer;
mod render_alloc;
mod restore;
mod shell;
mod surface;
