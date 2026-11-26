//! The match review: keys "Match on Steam" couldn't match exactly, each with Steam's closest
//! results in a searchable dropdown and a box to search Steam again with other words.

use rune::{El, ScrollSpec, TextEdit, col, el, row, text};
use rune_kit::{
    Button, ButtonVariant, Combobox, ComboboxOption, ComboboxState, Dialog, Kit, TextInput,
};

use super::{muted, primary};
use crate::app::{AppState, Screen, Session};

/// Review row `i`'s search box (the scratch buffer when it's gone).
fn term_edit(s: &mut AppState, i: usize) -> &mut TextEdit {
    let AppState {
        screen, scratch, ..
    } = s;
    match screen {
        Screen::Unlocked(x) => match x.match_review.as_mut().and_then(|r| r.rows.get_mut(i)) {
            Some(r) => &mut r.term,
            None => scratch,
        },
        Screen::Locked(_) => scratch,
    }
}

pub fn dialog(s: &AppState, sess: &Session, k: &Kit, overlays: &mut Vec<El>) {
    let Some(review) = &sess.match_review else {
        return;
    };
    let busy = s.steam_busy();
    let closed = ComboboxState::default();
    // Dropdowns open above the dialog, so they go into the overlays after it.
    let mut popups = Vec::new();
    let mut list = col().key("review-rows").w_full().gap(10.0);
    for (i, r) in review.rows.iter().enumerate() {
        let options: Vec<ComboboxOption> = r
            .matches
            .iter()
            .map(|m| {
                let mut o = ComboboxOption::new(m.app_id.to_string(), m.name.clone());
                o.detail = m.app_id.to_string();
                o
            })
            .collect();
        let state = if review.open == Some(i) {
            &s.match_combo
        } else {
            &closed
        };
        let chosen = r.chosen.map(|a| a.to_string());
        let pick = Combobox::new(
            format!("review-pick-{i}"),
            state,
            &options,
            move |s: &mut AppState| s.review_combo(i),
        )
        .label(format!("Steam game for {}", r.name))
        .placeholder(if r.matches.is_empty() {
            "No results — search again"
        } else {
            "Pick the game…"
        })
        .search_placeholder("Filter results…")
        .empty_message("Nothing matches; try Search with other words")
        .current(chosen.as_deref())
        .width(360.0)
        .disabled(r.matches.is_empty())
        .on_choose(move |s: &mut AppState, id: &str, _| s.review_choose(i, id))
        .build(k, &mut popups);
        let n = r.key_ids.len();
        list = list.child(
            col()
                .key(format!("review-row-{i}"))
                .w_full()
                .gap(6.0)
                .padding(10.0)
                .radius(k.radius.md)
                .fill(k.c.surface_raised)
                .child(
                    row()
                        .w_full()
                        .gap(8.0)
                        .center_y()
                        .child(
                            text(r.name.clone())
                                .font(k.ty.ui_medium)
                                .color(k.c.text_strong)
                                .grow(1.0)
                                .w(0.0)
                                .truncate(),
                        )
                        .children((n > 1).then(|| muted(k, format!("{n} keys")))),
                )
                .child(
                    row()
                        .w_full()
                        .gap(8.0)
                        .center_y()
                        .child(
                            TextInput::new(&r.term)
                                .placeholder("Search Steam for…")
                                .width(Some(220.0))
                                .on_edit(move |s: &mut AppState| term_edit(s, i))
                                .build(k)
                                .key(format!("review-term-{i}")),
                        )
                        .child(
                            el().key(format!("review-search-{i}")).child(
                                Button::new("Search")
                                    .variant(ButtonVariant::Outline)
                                    .disabled(busy)
                                    .on_press(move |s: &mut AppState| s.review_search(i))
                                    .build(k),
                            ),
                        )
                        .child(el().grow(1.0).min_w(0.0).child(pick)),
                ),
        );
    }
    let chosen = review.rows.iter().filter(|r| r.chosen.is_some()).count();
    let footer = row()
        .w_full()
        .pl(16.0)
        .gap(8.0)
        .center_y()
        .child(
            muted(
                k,
                "Skipped keys keep no Steam id; select them later and use Match on Steam to retry.",
            )
            .grow(1.0)
            .w(0.0)
            .wrap(),
        )
        .child(
            el().key("review-close").child(
                Button::new("Close")
                    .variant(ButtonVariant::Neutral)
                    .on_press(|s: &mut AppState| s.close_review())
                    .build(k),
            ),
        )
        .child(primary(
            k,
            "review-apply",
            if chosen == 0 {
                "Apply".to_string()
            } else {
                format!("Apply {chosen}")
            },
            chosen > 0,
            |s, _| s.apply_review(),
        ));
    let body = col()
        .w_full()
        .gap(14.0)
        .child(
            el().w_full()
                .max_h(440.0)
                .scroll(ScrollSpec::default())
                .child(list),
        )
        .child(footer);
    Dialog::new(
        "match-review",
        format!(
            "Pick the Steam game for {} title{}",
            review.rows.len(),
            if review.rows.len() == 1 { "" } else { "s" }
        ),
    )
    .open(true)
    .description("Steam had no exact match for these. Pick from its closest results, or search again with other words.")
    .width(760.0)
    .child(body)
    .on_close(|s: &mut AppState| s.close_review())
    .build(k, overlays);
    overlays.extend(popups);
}
