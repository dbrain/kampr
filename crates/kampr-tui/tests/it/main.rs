#[global_allocator]
static ALLOCATOR: kampr_testkit::alloc::Counting = kampr_testkit::alloc::Counting;

mod alerts;
mod copy;
mod desk;
mod images;
mod keys;
mod links;
mod manage;
mod pairing;
mod pictures;
mod pointer;
mod remember;
mod render_alloc;
mod restore;
mod route;
mod shell;
mod surface;
