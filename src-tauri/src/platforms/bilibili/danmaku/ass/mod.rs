use super::proto::DanmakuElem;

pub mod lanes;
mod render;

pub use render::{ass_time, AssRenderOptions};

pub fn render(elems: &[DanmakuElem], opts: &AssRenderOptions) -> String {
    render::render_ass(elems, opts)
}
