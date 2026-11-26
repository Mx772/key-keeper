//! Import wizard: choose a file or paste, check the column mapping, preview, import.

use keykeeper_core::DateFormat;
use keykeeper_core::import::{Field, RowOutcome};
use rune::{El, ScrollSpec, Tx, col, el, row, text};
use rune_kit::{Button, ButtonVariant, Checkbox, Kit, Select, TextArea};

use super::{card, hstack, muted, primary, sess_edit, spacer, status_pill, title_style};
use crate::app::{AppState, Session};
use crate::form::date_hint;

const PREVIEW_LIMIT: usize = 300;

pub fn view(_s: &AppState, sess: &Session, k: &Kit, overlays: &mut Vec<El>) -> El {
    let ui = &sess.import;
    let header = col()
        .gap(4.0)
        .child(
            text("Import keys")
                .font(title_style())
                .color(k.c.text_strong),
        )
        .child(muted(
            k,
            "CSV or TSV exports, an Excel copy-paste, or a plain list like \"Game: KEY\".",
        ));
    let mut page = col()
        .key("import")
        .w_full()
        .h_full()
        .px(32.0)
        .pt(28.0)
        .pb(20.0)
        .gap(20.0)
        .child(header);
    match (&ui.table, &ui.preview) {
        (Some(table), Some(preview)) => {
            page = page.child(summary_bar(sess, preview, k, overlays));
            page = page.child(mapping(sess, table, k, overlays));
            page = page.child(rows(sess, table, preview, k));
        }
        _ => page = page.child(choose(sess, k)),
    }
    page
}

fn choose(sess: &Session, k: &Kit) -> El {
    let file = card(
        k,
        "import-file",
        "From a file",
        col()
            .gap(12.0)
            .child(muted(k, "Pick a .csv, .tsv or .txt file. Columns are matched automatically and you can adjust them before anything is added.").wrap())
            .child(row().child(primary(k, "import-choose", "Choose file…", true, |s, tx| s.import_pick_file(tx)))),
    );
    let paste = card(
        k,
        "import-paste",
        "Paste",
        col()
            .gap(12.0)
            .child(
                TextArea::new(&sess.import.paste)
                    .placeholder("Paste rows from Excel / Sheets, or lines like\nUndertale: UNDR7-TALE0-00001")
                    .rows(8)
                    .width(None)
                    .on_edit(|s: &mut AppState| sess_edit(s, |x| &mut x.import.paste))
                    .build(k)
                    .key("import-paste-text"),
            )
            .child(row().child(spacer()).child(el().key("import-preview").child(Button::new("Preview").variant(ButtonVariant::Outline).on_press(|s: &mut AppState| s.import_paste()).build(k)))),
    );
    row()
        .w_full()
        .gap(20.0)
        .child(file.grow(1.0).w(0.0))
        .child(paste.grow(1.0).w(0.0))
}

fn summary_bar(
    sess: &Session,
    p: &keykeeper_core::import::ImportPreview,
    k: &Kit,
    overlays: &mut Vec<El>,
) -> El {
    let ui = &sess.import;
    let (new, dups, errs) = (p.new_count(), p.duplicate_count(), p.error_count());
    let adding = new + if ui.include_duplicates { dups } else { 0 };
    let badge = |label: String, fg| {
        el().h(24.0)
            .px(10.0)
            .radius(12.0)
            .center()
            .fill(k.c.surface_raised)
            .child(text(label).font(k.ty.text_small_medium).color(fg))
    };
    let formats = [DateFormat::MonthDayYear, DateFormat::DayMonthYear];
    let labels: Vec<&str> = formats.iter().map(|f| date_hint(*f)).collect();
    let date = Select::new("import-date-format", &labels)
        .label("Dates")
        .current(Some(date_hint(ui.date_format)))
        .open(ui.date_select_open)
        .width(140.0)
        .on_toggle(|s: &mut AppState| {
            if let Some(x) = s.session_mut() {
                x.import.date_select_open = !x.import.date_select_open;
            }
        })
        .on_close(|s: &mut AppState| {
            if let Some(x) = s.session_mut() {
                x.import.date_select_open = false;
            }
        })
        .on_choose(move |s: &mut AppState, v: &str, _: &mut Tx| {
            if let Some(f) = formats.into_iter().find(|f| date_hint(*f) == v) {
                s.set_import_date_format(f);
            }
        })
        .build(k, overlays);
    row()
        .key("import-summary")
        .w_full()
        .gap(10.0)
        .center_y()
        .child(
            text(ui.source.clone().unwrap_or_default())
                .font(k.ty.ui_medium)
                .color(k.c.text_strong),
        )
        .child(badge(format!("{new} new"), k.c.success.icon).key("import-new"))
        .child(badge(format!("{dups} duplicate"), k.c.text_muted).key("import-dups"))
        .child(
            badge(
                format!("{errs} skipped"),
                if errs > 0 {
                    k.c.warning.icon
                } else {
                    k.c.text_muted
                },
            )
            .key("import-errors"),
        )
        .child(spacer())
        .child(date)
        .child(
            Checkbox::new(ui.include_duplicates)
                .label("Include duplicates")
                .disabled(dups == 0)
                .on_toggle(|s: &mut AppState| {
                    if let Some(x) = s.session_mut() {
                        x.import.include_duplicates = !x.import.include_duplicates;
                    }
                })
                .build(k),
        )
        .child(
            Button::new("Start over")
                .variant(ButtonVariant::Neutral)
                .on_press(|s: &mut AppState| s.reset_import())
                .build(k),
        )
        .child(primary(
            k,
            "import-commit",
            format!("Import {adding} key{}", if adding == 1 { "" } else { "s" }),
            adding > 0,
            |s, _| s.commit_import(),
        ))
}

fn mapping(
    sess: &Session,
    table: &keykeeper_core::import::Table,
    k: &Kit,
    overlays: &mut Vec<El>,
) -> El {
    let mut options: Vec<&str> = vec!["Ignore"];
    options.extend(Field::ALL.iter().map(|f| f.label()));
    let mut cols = row().key("import-mapping").gap(12.0);
    for (i, header) in table.headers.iter().enumerate() {
        let current = table
            .mapping
            .get(i)
            .copied()
            .flatten()
            .map_or("Ignore", Field::label);
        let select = Select::new(format!("map-{i}"), &options)
            .current(Some(current))
            .open(sess.import.open_column == Some(i))
            .width(170.0)
            .on_toggle(move |s: &mut AppState| {
                if let Some(x) = s.session_mut() {
                    x.import.open_column = if x.import.open_column == Some(i) {
                        None
                    } else {
                        Some(i)
                    };
                }
            })
            .on_close(|s: &mut AppState| {
                if let Some(x) = s.session_mut() {
                    x.import.open_column = None;
                }
            })
            .on_choose(move |s: &mut AppState, v: &str, _: &mut Tx| {
                let field = Field::ALL.into_iter().find(|f| f.label() == v);
                s.set_column_mapping(i, field);
            })
            .build(k, overlays);
        let sample = table
            .rows
            .first()
            .and_then(|r| r.get(i))
            .cloned()
            .unwrap_or_default();
        let sample = if table.mapping.get(i).copied().flatten() == Some(Field::Key) {
            keykeeper_core::Secret::new(sample).masked()
        } else {
            sample
        };
        cols = cols.child(
            col()
                .w(170.0)
                .gap(6.0)
                .child(
                    text(if header.is_empty() {
                        format!("Column {}", i + 1)
                    } else {
                        header.clone()
                    })
                    .font(k.ty.text_small_medium)
                    .color(k.c.text_strong)
                    .truncate(),
                )
                .child(select)
                .child(
                    text(sample)
                        .font(k.ty.text_xsmall)
                        .color(k.c.text_faint)
                        .truncate(),
                ),
        );
    }
    card(
        k,
        "import-columns",
        "Columns",
        col().w_full().scroll(ScrollSpec::default()).child(cols),
    )
}

fn rows(
    sess: &Session,
    table: &keykeeper_core::import::Table,
    p: &keykeeper_core::import::ImportPreview,
    k: &Kit,
) -> El {
    let _ = sess;
    // For rows that produced no key (errors), still show the name the file had.
    let name_col = table
        .mapping
        .iter()
        .position(|m| *m == Some(Field::GameName));
    let raw_name = |row: usize| {
        name_col
            .and_then(|c| table.rows.get(row).and_then(|r| r.get(c)))
            .map(|s| s.trim().to_string())
            .unwrap_or_default()
    };
    let mut list = col().key("import-rows").w_full().gap(2.0);
    for r in p.rows.iter().take(PREVIEW_LIMIT) {
        let (badge, color) = match &r.outcome {
            RowOutcome::New => ("New", k.c.success.icon),
            RowOutcome::Duplicate {
                existing: Some(_), ..
            } => ("In vault", k.c.text_muted),
            RowOutcome::Duplicate { .. } => ("Repeated", k.c.text_muted),
            RowOutcome::Error(_) => ("Skipped", k.c.warning.icon),
        };
        let (name, code, status) = match &r.key {
            Some(key) => (key.game_name.clone(), key.key.masked(), Some(key.status)),
            None => (raw_name(r.row), String::new(), None),
        };
        let mut notes: Vec<String> = r.warnings.clone();
        if let RowOutcome::Error(e) = &r.outcome {
            notes.insert(0, e.clone());
        }
        let mut line = row()
            .w_full()
            .h(34.0)
            .px(12.0)
            .gap(12.0)
            .center_y()
            .radius(k.radius.sm)
            .fill(k.c.bg_base)
            .child(
                text(format!("{}", r.line))
                    .font(k.ty.text_xsmall)
                    .color(k.c.text_faint)
                    .w(36.0),
            )
            .child(
                text(badge)
                    .font(k.ty.text_small_medium)
                    .color(color)
                    .w(70.0),
            )
            .child(
                text(name)
                    .font(k.ty.text_small)
                    .color(k.c.text_base)
                    .w(240.0)
                    .truncate(),
            )
            .child(
                text(code)
                    .font(k.ty.mono)
                    .color(k.c.text_muted)
                    .w(200.0)
                    .truncate(),
            );
        line = match status {
            Some(st) => line.child(el().w(100.0).child(status_pill(k, st))),
            None => line.child(el().w(100.0)),
        };
        line = line.child(
            text(notes.join(" · "))
                .font(k.ty.text_small)
                .color(k.c.warning.icon)
                .grow(1.0)
                .truncate(),
        );
        list = list.child(line);
    }
    if p.rows.len() > PREVIEW_LIMIT {
        list = list.child(
            muted(
                k,
                format!("…and {} more rows", p.rows.len() - PREVIEW_LIMIT),
            )
            .pt(6.0),
        );
    }
    col()
        .key("import-preview-frame")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .gap(10.0)
        .child(
            hstack().child(
                text("Preview")
                    .font(super::heading_style())
                    .color(k.c.text_strong),
            ),
        )
        .child(
            col()
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec::default())
                .child(list),
        )
}
