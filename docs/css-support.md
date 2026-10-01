# CSS and HTML support

Erk supports a deliberately bounded subset of HTML and CSS, chosen for desktop
UI. This page says what works today, what is planned and when, and what is
**not planned**. If a feature is not listed here, assume it is not supported.

- **Supported**: works today. Every row names the test that proves it; from M1,
  CI checks that the named test exists.
- **M1**, **M2**, …: planned for that milestone ([roadmap](plans/roadmap.md),
  Turkish).
- **Not planned**: out of scope, with the reason. Moving a feature out of this
  list is a design decision, written down under `docs/design/` first.

Styling is done by Stylo, so parsing and the cascade follow the standards.
Support below means **layout and painting**: a property Stylo computes but Erk
does not lay out or paint is not supported.

## Scripting

| Feature | Status | Notes |
|---|---|---|
| `<script>` and scripts the host runs | Later (M6) | Off by default. The engine core runs no scripts; the optional `erk-script` binding runs JavaScript on top of the public API, with a small DOM subset (selectors, text, attributes, `classList`, `style`, create/insert/remove, `addEventListener`) |
| Event handler attributes (`onclick`, …), `javascript:` URLs | Not planned | Listen with `addEventListener` in `erk-script`, or subscribe from the host through the embedding API |
| Web APIs (`fetch`, storage, workers, `XMLHttpRequest`) | Not planned | A script reaches files or the network only through functions the host exposes |

## Selectors and cascade

| Feature | Status | Test / notes |
|---|---|---|
| Type, class and id selectors | Supported | `erk-style/tests/computed.rs`: `class_and_id_selectors_match` |
| `<style>` blocks, `style` attribute, inheritance | Supported | `author_stylesheet_applies`, `style_attribute_applies`, `inherited_properties_flow_down` |
| Custom properties (`--name`, `var()`), which utility CSS such as Tailwind relies on | Supported | `css_custom_properties_resolve` |
| User agent stylesheet (headings, block elements) | Supported | `user_agent_stylesheet_makes_headings_blocks_with_larger_text` |
| `:hover`, `:active`, `:focus` | M2 | Element state from the input pipeline |
| `:focus-visible`, form pseudo-classes (`:checked`, `:disabled`) | M5 | With form controls |

## Box model and layout

| Feature | Status | Test / notes |
|---|---|---|
| Block layout: stacking, width, margin, padding, border widths | Supported | `erk-renderer/src/layout/tests.rs`: `block_siblings_stack_vertically`, `explicit_width_is_used`, `padding_and_border_widen_the_border_box` |
| Vertical margin collapsing | Supported | `adjacent_vertical_margins_collapse` |
| Text beside block children (anonymous boxes) | Supported | `text_beside_blocks_gets_anonymous_boxes`, `whitespace_between_blocks_makes_no_anonymous_box` |
| `calc()` lengths | Supported | `calc_widths_resolve_against_the_container` |
| `ex`, `ch` units | Supported | `ex_and_ch_come_from_the_embedded_font` |
| `display: none`, `visibility: hidden` | Supported | `display_none_generates_no_box`, `visibility_hidden_paints_neither_background_nor_text` |
| Inline elements keep their own colour, weight, size and line height within a paragraph | Supported | `an_inline_element_keeps_its_own_weight`, `an_inline_element_keeps_its_own_colour` |
| Inline elements' horizontal padding, border and margin take room in the line; their background is painted per line, around the font's ascent and descent plus vertical padding | Supported | `inline_padding_border_and_margin_take_room_in_the_line`, `inline_padding_extends_the_background_but_not_the_line`, `a_wrapped_inline_background_gets_one_rectangle_per_line`, `an_inline_background_lies_between_its_block_and_its_text`. Percentages resolve to zero; a padded element's ends are line-break opportunities |
| `inline-block`, `inline-flex`, `inline-grid` (atomic inlines) on the line's baseline | Supported | `an_inline_block_is_laid_out_and_sits_on_the_baseline`, `an_inline_block_with_text_aligns_its_text_with_the_line`, `text_after_a_tall_line_moves_down`. A block-container inline-block uses its first line's baseline where CSS uses the last |
| `vertical-align` | M1 | |
| Inline images | M1 | With `<img>` (below) |
| Flexbox | M1 | Laid out by Taffy today, not yet verified |
| `position: absolute`, `relative`, `fixed` | M1 | |
| `overflow: auto`, `scroll`, scroll containers | M2 | |
| `contain: size layout paint` | M5 | A relayout and repaint boundary for incremental rendering |
| `content-visibility` | Later | |
| Grid | Later | Laid out by Taffy; verified after flexbox |
| `float`, `clear` | Not planned | Desktop UI is built with flexbox; floats are the most edge-case-heavy part of CSS. A floated element is laid out as if `float: none` (M1) |
| Table layout (`display: table`, `<table>` as a grid of cells) | Not planned | Use grid or flexbox. Tables parse and are styled, but there is no table layout algorithm |
| Multi-column layout | Not planned | Not a UI layout |
| Print and paged media (`@page`, page breaks) | Not planned | Erk renders to windows, not pages |
| Vertical `writing-mode` | Not planned | |

## Text and fonts

| Feature | Status | Test / notes |
|---|---|---|
| `color`, `font-size`, `font-weight` (regular and bold) | Supported | `bold_uses_the_bold_face`, `headings_are_larger_than_paragraphs` |
| Line breaking, whitespace collapsing, `line-height: normal` | Supported | `narrow_width_breaks_into_more_lines`, `whitespace_collapses_and_trims`, `a_paragraph_is_one_line_high` |
| Turkish and other Latin text | Supported | `turkish_letters_all_have_glyphs` |
| `text-align` (`start`, `end`, `left`, `right`, `center`, `justify`), `align` attribute | Supported | `text_align_moves_the_line_within_the_box`, `justified_lines_fill_the_box_except_the_last`, `the_align_attribute_aligns_text` |
| System fonts and font fallback (CJK, emoji) | M1 | Today only the embedded Noto Sans |
| `text-transform` with the element's `lang` (Turkish `i → İ`) | M1 | |
| Text selection, caret, IME input | M5 | |
| Web fonts (`@font-face` from the host's resources) | Later | |
| `::first-line`, `::first-letter` | Not planned | Rarely used in UI, costly in inline layout |

## Backgrounds, borders, images

| Feature | Status | Test / notes |
|---|---|---|
| `background-color`, canvas background propagation, `bgcolor` attribute | Supported | `a_translucent_canvas_is_blended_over_white`, `an_element_without_a_box_does_not_colour_the_canvas`, `bgcolor_sets_the_background_colour` |
| Border colour and style (solid), `border-radius` | M1 | Border widths take space today but are not painted |
| `box-shadow` | M1 | |
| `<img>` and `background-image` (PNG, JPEG) through the host's resource callback | M1 | Erk never reads files itself |
| Gradients | Later | |

## Effects and animation

| Feature | Status | Test / notes |
|---|---|---|
| `opacity` | M1 | |
| `transform` (2D) | Later | |
| Transitions and animations | M9 | Driven by the host's clock |
| `filter`, `backdrop-filter`, `mix-blend-mode` | Not planned | Costly compositing effects; revisit only with the compositor (M9) |
| `shape-outside`, `clip-path` | Not planned | |

## Forms

| Feature | Status | Test / notes |
|---|---|---|
| `input` (text, checkbox, radio), `textarea`, `button`, `select` | M5 | Native to Erk, with focus, selection, clipboard and IME |
| Form submission to a URL | Not planned | There is no network; the host receives a submit event |

## Media and other HTML

| Feature | Status | Test / notes |
|---|---|---|
| HTML parsing (full HTML5 algorithm) | Supported | `erk-dom/tests/parse.rs` |
| `<details>`/`<summary>`, `<dialog>`, the `popover` attribute, `commandfor`/`command` | Later (M5) | Built-in behaviour: menus, disclosure widgets and dialogs work without script or a host round trip |
| `<audio>`, `<video>`, `<canvas>`, `<iframe>`, SVG | Not planned for now | Revisited after M9 |
