//! Render a page to a PNG: the smallest use of Erk today, and the example in
//! the README. Run with `cargo run -p erk-renderer --example png`.

use erk_renderer::render_html;

const PAGE: &str = r#"<!DOCTYPE html>
<style>
  body { margin: 24px; font-family: "Noto Sans"; color: #1f2933 }
  .tag { background: #bfdbfe; padding: 2px 8px }
  .button { display: inline-block; background: #1d4ed8; color: #fff; padding: 6px 14px }
</style>
<p>Erk paints <span class="tag">HTML and CSS</span> without a browser.</p>
<p>Inline blocks sit on the baseline: <span class="button">Save</span></p>"#;

fn main() {
    let frame = render_html(PAGE, 480, 140);
    let png = frame.to_png().expect("the frame has pixels");
    std::fs::write("erk.png", png).expect("erk.png can be written");
    println!("wrote erk.png ({}x{})", frame.width(), frame.height());
}
