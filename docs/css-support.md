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
| The host finds elements with CSS selectors and sets their text (`erk_query`, `erk_node_set_text`) | Supported | `queries_find_what_css_selectors_match_in_document_order`, `a_query_finds_the_first_match_or_says_what_is_wrong`, `set_text_changes_the_page_and_a_stale_node_is_an_error`, `clicks_count_and_the_page_shows_the_number`. A removed node, or one of a document since replaced, is an error, never another node. More changes (attributes, classes, inserting and removing) come in M4 |
| Event handler attributes (`onclick`, …), `javascript:` URLs | Not planned | Listen with `addEventListener` in `erk-script`, or subscribe from the host through the embedding API |
| Web APIs (`fetch`, storage, workers, `XMLHttpRequest`) | Not planned | A script reaches files or the network only through functions the host exposes |

## Selectors and cascade

| Feature | Status | Test / notes |
|---|---|---|
| Type, class and id selectors | Supported | `erk-style/tests/computed.rs`: `class_and_id_selectors_match` |
| `<style>` blocks, `style` attribute, inheritance | Supported | `author_stylesheet_applies`, `style_attribute_applies`, `inherited_properties_flow_down` |
| Custom properties (`--name`, `var()`), which utility CSS such as Tailwind relies on | Supported | `css_custom_properties_resolve` |
| User agent stylesheet (headings, block elements) | Supported | `user_agent_stylesheet_makes_headings_blocks_with_larger_text` |
| `:hover`, `:active`, `:focus`, `:focus-within` | Supported | `hover_active_and_focus_match_the_element_the_user_points_at`, `hover_and_active_restyle_the_element_under_the_pointer`, `the_states_reference_page_follows_the_pointer_and_the_focus`. A state change restyles the whole page; a page whose selectors do not use a state is not painted again when it changes (`styles_say_which_states_their_selectors_depend_on`) |
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
| `vertical-align`: `baseline`, `sub`, `super`, lengths, percentages, `middle`, `text-top`, `text-bottom` on inline elements and atomic inlines; `top`, `center`, `bottom` on atomic inlines | Supported | `sup_and_sub_move_their_text_off_the_baseline`, `raised_and_lowered_text_make_room_on_their_own_side`, `vertical_align_places_an_inline_block_against_the_parent`, `vertical_align_top_and_bottom_follow_the_line_box`, `superscript_glyphs_are_painted_raised`. On a non-atomic inline element the line-relative values (top, center, bottom) are laid out as baseline |
| Inline images | Supported | `an_img_takes_its_natural_size_and_is_painted`: an atom standing on the baseline |
| Flexbox: `flex-direction`, `flex-wrap`, `gap`, `justify-content`, `align-items`, `flex-grow`/`shrink`/`basis`, `order`; text in a flex container as anonymous items | Supported | `flex_grow_shrink_basis_and_gap_share_the_line`, `wrapping_columns_and_alignment_follow_the_container`, `order_rearranges_flex_items`, `text_alone_in_a_flex_container_is_a_flex_item`. Laid out by Taffy; items paint in document order rather than order-modified order |
| `position: relative`, `absolute`, `fixed` with `top`, `right`, `bottom`, `left`; the containing block is the nearest positioned ancestor or the viewport | Supported | `relative_position_offsets_the_box_but_not_the_flow`, `an_absolute_box_is_placed_in_its_nearest_positioned_ancestor`, `without_a_positioned_ancestor_the_viewport_contains`, `a_fixed_box_ignores_its_positioned_ancestors`, `an_absolute_element_does_not_split_its_paragraph`, `static_position_ignores_insets`, `an_absolute_block_with_auto_insets_sits_where_it_would_have_been`, `an_absolute_inline_sits_at_its_place_in_the_line`, `a_flex_container_places_its_absolute_children`. An inline element is never a containing block |
| `z-index` and paint order of positioned elements (CSS 2 Appendix E) | Supported | `positioned_boxes_paint_after_the_flow`, `z_index_orders_positioned_boxes`. Each positioned element paints as a unit; positioned descendants of a z-index auto element stay inside it |
| `position: sticky` | Later | Laid out as `static` until scrolling exists (M2) |
| `overflow: hidden`, `clip`, `auto`, `scroll`: content clipped to the padding box | Supported | `overflow_clips_what_a_box_holds_to_its_padding_box`, `clips_nest`, `text_that_overflows_is_clipped`, `positioned_boxes_escape_a_clip_their_containing_block_is_outside_of`, `positioned_and_translucent_content_is_clipped_where_it_is_painted`, `the_root_or_body_overflow_belongs_to_the_viewport`; the overflow reference page. The clip is rectangular: rounded corners (`border-radius`) do not round it yet |
| Scroll containers and the document scroll with the wheel, the innermost first, the rest passed to the one around it; overlay scroll bars while the pointer is over them | Supported | `the_wheel_scrolls_the_innermost_container_then_the_ones_around_it`, `hit_regions_move_with_the_content_and_are_clipped_with_it`, `scroll_bars_show_while_the_pointer_is_over_what_they_scroll`, `a_long_page_scrolls_with_the_wheel`, `text_directly_in_a_scroll_container_scrolls_with_it`. The scroll range covers the border boxes and the lines of text inside. `overflow: scroll` reserves no room for a scroll bar |
| Scrolling from the keyboard, `scroll-behavior`, `scroll-snap`, `overscroll-behavior`, scrolling an element into view | Later | |
| `contain: size layout paint` | M5 | A relayout and repaint boundary for incremental rendering |
| `content-visibility` | Later | |
| Grid | Later | Laid out by Taffy; verified after flexbox |
| `float`, `clear` | Not planned | Desktop UI is built with flexbox; floats are the most edge-case-heavy part of CSS. A floated element is laid out as if `float: none` (test: a_float_is_laid_out_as_if_not_floated) |
| Table layout (`display: table`, `<table>` as a grid of cells) | Not planned | Use grid or flexbox. Tables parse and are styled, but there is no table layout algorithm |
| Multi-column layout | Not planned | Not a UI layout |
| Print and paged media (`@page`, page breaks) | Not planned | Erk renders to windows, not pages |
| Vertical `writing-mode` | Not planned | |

## Text and fonts

| Feature | Status | Test / notes |
|---|---|---|
| `color`, `font-size`, `font-weight` (regular and bold) | Supported | `bold_uses_the_bold_face`, `headings_are_larger_than_paragraphs` |
| `white-space: nowrap`, `pre`, `pre-wrap`, `pre-line`, `break-spaces` | M2 | Whitespace always collapses today; `css-text/white-space` passes 45 of 422. Which values come in M2.6 is decided from the tests |
| Line breaking, whitespace collapsing, `line-height: normal`; every line at least the block's strut | Supported | `narrow_width_breaks_into_more_lines`, `whitespace_collapses_and_trims`, `a_paragraph_is_one_line_high`, `smaller_text_at_the_end_of_a_line_does_not_shorten_it` |
| Turkish and other Latin text | Supported | `turkish_letters_all_have_glyphs` |
| `text-align` (`start`, `end`, `left`, `right`, `center`, `justify`), `align` attribute | Supported | `text_align_moves_the_line_within_the_box`, `justified_lines_fill_the_box_except_the_last`, `the_align_attribute_aligns_text` |
| `font-family`: the host's font families, and the generic families `serif`, `sans-serif`, `monospace`, `cursive`, `fantasy`, `system-ui` | Supported | `a_named_family_from_the_catalogue_draws_the_text`, `a_generic_family_maps_through_the_catalogue`, `a_family_the_page_does_not_use_is_not_requested`, `without_a_catalogue_every_family_is_the_embedded_font`. The host scans the system's fonts (p1-contract §6.2); without it every family is the embedded Noto Sans. Line heights and vertical-align still use Noto Sans's metrics |
| Font fallback by writing system and language (CJK, Arabic, Hebrew, Thai, ...), colour emoji | Supported | `characters_the_font_lacks_fall_back_by_script`, `the_text_language_selects_its_fallback`, `a_language_without_its_own_list_uses_the_list_for_any_language`, `emoji_are_drawn_with_the_emoji_family`. The fallback lists are the platform's |
| `font-style: italic`, `oblique` | Supported | `a_face_is_asked_for_by_family_weight_and_style`. With the host's fonts only: the embedded font has no italic and is drawn upright, where Chrome slants it |
| `direction`, bidirectional paragraphs | Later | Right-to-left text is shaped and drawn, but a paragraph takes its direction from its first strong character rather than from `direction`: Hebrew in a left-to-right box starts at the right (Parley 0.11 has no setting for it) |
| `text-transform: uppercase`, `lowercase`, `capitalize` in the language of the nearest `lang` attribute (Turkish `i → İ`, Greek capitals without accents, Dutch `IJ`) | Supported | `uppercase_and_lowercase_follow_the_language`, `the_nearest_lang_attribute_decides`, `capitalize_titlecases_the_first_letter_of_each_word`, `a_word_continues_across_inline_elements`, `only_the_transformed_element_changes`. Text without a language uses the root rules (Chrome uses its own UI language). Unlike Chrome, capitalize is language-sensitive too |
| `text-transform: full-width`, `full-size-kana` | Later | |
| Text selection, caret, IME input | M5 | |
| Web fonts (`@font-face` from the host's resources) | Later | |
| `::first-line`, `::first-letter` | Not planned | Rarely used in UI, costly in inline layout |

## Backgrounds, borders, images

| Feature | Status | Test / notes |
|---|---|---|
| `background-color`, canvas background propagation, `bgcolor` attribute | Supported | `a_translucent_canvas_is_blended_over_white`, `an_element_without_a_box_does_not_colour_the_canvas`, `bgcolor_sets_the_background_colour` |
| Borders: width, colour per side, on blocks and inline elements | Supported | `a_solid_border_is_painted_around_the_padding_box`, `each_border_side_keeps_its_own_colour`, `an_inline_border_closes_only_the_first_and_last_line`, `the_canvas_element_still_paints_its_border`. Every style other than none and hidden is drawn solid |
| Border styles `dotted`, `dashed`, `double`, `groove`, `ridge`, `inset`, `outset` | Later | Drawn solid |
| `border-radius`, circular and elliptical, percentages; the background is clipped to it | Supported | `rounded_corners_clip_the_background`, `overlapping_radii_are_scaled_down_together`. With overflow other than visible, children are clipped to the padding box, not yet to its rounded corners; an inline element's background stays square |
| `box-shadow` (outer: offset, blur, spread, several shadows) | Supported | `a_box_shadow_is_cast_outside_the_box_only`, `an_offset_blurred_shadow_lies_behind_the_background`. A rounded box's shadow uses one mean radius for its corners |
| `box-shadow: inset` | Later | |
| `<img>` (PNG, JPEG) through the host's resource callback: natural size, `width`/`height` attributes, the natural ratio with CSS sizes and min/max | Supported | `an_img_takes_its_natural_size_and_is_painted`, `one_given_dimension_keeps_the_natural_ratio`, `both_given_dimensions_win_over_the_ratio`, `an_image_that_never_arrives_has_no_size_and_paints_nothing`, `a_response_of_the_wrong_type_is_refused`. Erk never reads files itself |
| `background-image: url()` with `background-size` (`cover`, `contain`, lengths, `auto`), `background-position`, `background-repeat`; several layers; clipped to the border box and its radii | Supported | `a_background_image_repeats_from_the_padding_box`, `background_position_and_no_repeat_place_one_copy`, `background_size_cover_fills_the_box`. The space and round keywords tile like repeat |
| `background-origin`, `background-clip`, `background-attachment`, `object-fit`, `object-position` | Later | Origin is the padding box, clip the border box |
| GIF, WebP, SVG images, `data:` URLs | Later | |
| Gradients (`linear-gradient`, `radial-gradient`) | M4 | Brought forward from Later: modern buttons and cards use subtle gradients |

## Effects and animation

| Feature | Status | Test / notes |
|---|---|---|
| `opacity` | Supported | `opacity_composites_an_element_as_one_group`. The element paints as one group, ordered like a positioned element with z-index 0 |
| `transform` (2D: `translate`, `scale`, `rotate`) | M4 | Brought forward from Later: the press and hover feedback of modern UI; hit-testing follows the transform |
| Transitions of `color`, `background-color`, `opacity`, `transform` | M5 | Brought forward from M9: linear interpolation with the standard timing functions, driven by the host's clock |
| `@keyframes` animations, transitions run on the compositor | M9 | Driven by the host's clock |
| `backdrop-filter: blur` (frosted glass) | M9 | Common behind sidebars and dialogs; blurring what lies behind needs the compositor's layers |
| `filter`, other `backdrop-filter` functions, `mix-blend-mode` | Not planned | Costly compositing effects; revisit only with the compositor (M9) |
| `shape-outside`, `clip-path` | Not planned | |

## Interaction

| Feature | Status | Test / notes |
|---|---|---|
| Hit-testing: the topmost box under the pointer in paint order (stacking, `z-index`), text targets its element, `pointer-events: none` passes through, `visibility: hidden` is no target | Supported | `a_click_reports_the_element_under_the_pointer_and_its_path`, `a_click_on_text_targets_the_element_the_text_is_in`, `the_topmost_target_wins_and_untargetable_boxes_are_passed_through`. A point in the viewport no box covers belongs to the root element, as in browsers |
| Click events with the path from the target to the root element, for the host to capture and bubble | Supported | `a_press_and_a_release_on_different_elements_click_their_common_ancestor`, `only_a_press_and_release_of_the_primary_button_click`. A press and release on different elements click their deepest common ancestor |
| Inspecting: the node at a point, a highlight drawn over a node's boxes | Supported | `inspect_at_answers_the_topmost_node_or_none`, `the_highlight_is_drawn_over_the_page_and_not_into_it` |
| Focus: a press focuses the focusable element pressed (links, enabled controls, `tabindex`), Tab and Shift+Tab walk the sequential focus order, with focus and blur events | Supported | `a_press_moves_the_focus_and_reports_blur_then_focus`, `tab_walks_the_focus_order_and_shift_tab_walks_it_back`. The order wraps around at the ends; Tab from an element outside the order (a negative tabindex) starts from the first |
| Keyboard activation: Enter clicks a focused link or button, Space released clicks a focused button | Supported | `enter_and_space_click_the_focused_link_or_button_as_browsers_do` |
| Key events to the host (`keydown`, `keyup`), text input | M5 | With form controls; keys the engine does not act on are ignored until then |
| `cursor` keywords; `auto` is a text cursor over text; links show the hand | Supported | `the_cursor_follows_what_the_pointer_is_over`. Cursor images (`url()`) fall back to their keyword |
| Other `pointer-events` values (SVG) | Not planned | Only `auto` and `none` apply to HTML boxes |

## Forms

| Feature | Status | Test / notes |
|---|---|---|
| `input` (text, checkbox, radio), `textarea`, `button`, `select` | M5 | Native to Erk, with focus, selection, clipboard and IME |
| Form submission to a URL | Not planned | There is no network; the host receives a submit event |

## Media and other HTML

| Feature | Status | Test / notes |
|---|---|---|
| HTML parsing (full HTML5 algorithm) | Supported | `erk-dom/tests/parse.rs` |
| HiDPI: layout in CSS pixels, painting at the screen's device scale; `resolution` media queries | Supported | `a_css_pixel_covers_scale_device_pixels`, `the_viewport_is_the_device_size_in_css_pixels`, `text_is_drawn_at_device_resolution`, `resolution_media_queries_see_the_device_scale`. Positions round to CSS pixels, so a fractional scale leaves some edges soft |
| `<details>`/`<summary>`, `<dialog>`, the `popover` attribute, `commandfor`/`command` | Later (M5) | Built-in behaviour: menus, disclosure widgets and dialogs work without script or a host round trip |
| `<audio>`, `<video>`, `<canvas>`, `<iframe>`, SVG | Not planned for now | Revisited after M9 |
