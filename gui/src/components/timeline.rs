//! 时间线：记录按日期分组（今天/昨天/…），新→旧。

use std::rc::Rc;

use nsh_core::prelude::*;

use crate::state::{AppState, NeverEq};
use crate::components::record_card::{TRow, record_row};
use crate::palette as p;

/// 时间线主体。
pub fn timeline(state: &Rc<AppState>, mobile: bool) -> View {
    let state = state.clone();
    let records = state.records.clone();
    view()
        .flex_1()
        .px(px(if mobile { 20.0 } else { 0.0 }))
        .child(
            scroll()
                .flex_1()
                .child(
                    view()
                        .max_w(px(888.0))
                        .w_full()
                        .self_center()
                        .py(px(8.0))
                        .px(px(if mobile { 0.0 } else { 24.0 }))
                        .gap(px(14.0))
                        .child(each(
                            move || group_rows(records.get()),
                            |row: &NeverEq<TRow>| row_key(&row.0),
                            move |row, _| record_row(&state, row, mobile),
                        ))
                        .child(when(
                            {
                                let records = records.clone();
                                move || records.get().is_empty()
                            },
                            || empty_filtered_hint(),
                        )),
                ),
        )
}

/// 分组：按本地日期插入 Day 行。
fn group_rows(records: Vec<drift::Record>) -> Vec<NeverEq<TRow>> {
    let mut out: Vec<NeverEq<TRow>> = Vec::new();
    let mut last_day: Option<String> = None;
    for r in records {
        let label = crate::state::day_label(r.timestamp);
        if last_day.as_deref() != Some(label.as_str()) {
            out.push(NeverEq(TRow::Day(label.clone())));
            last_day = Some(label);
        }
        out.push(NeverEq(TRow::Rec(r)));
    }
    out
}

fn row_key(row: &TRow) -> String {
    match row {
        TRow::Day(label) => format!("day:{label}"),
        TRow::Rec(r) => format!("rec:{}", r.id),
    }
}

fn empty_filtered_hint() -> View {
    view()
        .py(px(40.0))
        .items_center()
        .child(
            text("没有匹配的记录")
                .text_size(px(12.0))
                .text_color(p::FAINT),
        )
}
