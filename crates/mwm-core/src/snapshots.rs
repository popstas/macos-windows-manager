//! Снимки раскладки: когда снимать, что снимать, сколько хранить.
//!
//! Перенос логики, отстоявшейся в windows11-manager: списки снимков лежат в
//! одном режиме пикера, и разное поведение двух машин читалось бы как поломка.

use crate::geometry::Bounds;
use crate::state::SlotState;
use std::collections::BTreeMap;

/// Сколько состав должен продержаться, чтобы стать снимком.
pub const DEBOUNCE_MS: u64 = 60_000;

/// Сколько снимков хранится. Дальше — вытесняются с хвоста.
pub const KEEP: usize = 20;

#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotSession {
    pub id: String,
    pub title: String,
    pub cwd: String,
    pub bounds: Bounds,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub id: String,
    pub created_s: u64,
    pub updated_s: u64,
    pub sessions: Vec<SnapshotSession>,
}

/// Что снапшотер должен сделать на этом такте.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Состав устоялся после изменения — новый снимок.
    Append,
    /// Состав тот же, съехали координаты — переписать последний.
    Update,
}

/// Ключ состава: отсортированные id через разделитель.
///
/// Сортировка обязательна: порядок сессий задан обходом окон и меняется сам по
/// себе, а ключ, зависящий от него, объявлял бы новый состав на ровном месте.
pub fn composition_key(ids: &[String]) -> String {
    let mut sorted: Vec<&str> = ids.iter().map(|s| s.as_str()).collect();
    sorted.sort_unstable();
    sorted.join("\u{1}")
}

/// Пустой состав не снимается вовсе: закрыл всё на ночь — наутро
/// восстанавливается последний рабочий набор, а не пустота.
pub fn decide(
    key: &str,
    last_key: &str,
    pending_key: &str,
    pending_since_ms: u64,
    now_ms: u64,
    debounce_ms: u64,
) -> Option<Decision> {
    if key.is_empty() {
        return None;
    }
    // Состав совпадает с последним снимком: остаются только координаты.
    if key == last_key {
        return if !pending_key.is_empty() && pending_key != key {
            None
        } else {
            Some(Decision::Update)
        };
    }
    // Состав другой — ждём, пока он устоится.
    if pending_key != key {
        return None;
    }
    if now_ms.saturating_sub(pending_since_ms) >= debounce_ms {
        Some(Decision::Append)
    } else {
        None
    }
}

/// Таймер перезапускается на каждое новое значение ключа: снимок фиксирует не
/// момент изменения, а устоявшееся состояние.
pub fn track_composition(
    key: &str,
    pending_key: &str,
    pending_since_ms: u64,
    now_ms: u64,
) -> (String, u64) {
    if key == pending_key {
        (pending_key.to_string(), pending_since_ms)
    } else {
        (key.to_string(), now_ms)
    }
}

/// Состав снимка из открытых сессий. Сессия без координат пропускается:
/// восстанавливать у неё нечего, а строка в списке появилась бы.
pub fn sessions_of(open: &[String], slots: &BTreeMap<String, SlotState>) -> Vec<SnapshotSession> {
    let mut ids: Vec<&String> = open.iter().collect();
    ids.sort_unstable();
    ids.dedup();
    ids.into_iter()
        .filter_map(|sid| {
            let s = slots.get(sid)?;
            Some(SnapshotSession {
                id: sid.clone(),
                title: s.title.clone(),
                cwd: s.cwd.clone(),
                bounds: s.bounds?,
            })
        })
        .collect()
}

/// Новый снимок в голову списка, лишние вытесняются с хвоста.
///
/// `keep == 0` читается как «настройки нет», а не как «не хранить ничего», —
/// подставляется умолчание `KEEP`. Числом хранимых снимков снимки не
/// выключают: человек угадывал бы этот способ, а не читал его, а
/// выключения снимков на этом этапе нет вовсе.
pub fn append(
    snapshots: Vec<Snapshot>,
    id: String,
    sessions: Vec<SnapshotSession>,
    now_s: u64,
    keep: usize,
) -> Vec<Snapshot> {
    let keep = if keep == 0 { KEEP } else { keep };
    let mut out = Vec::with_capacity(snapshots.len() + 1);
    out.push(Snapshot { id, created_s: now_s, updated_s: now_s, sessions });
    out.extend(snapshots);
    out.truncate(keep);
    out
}

/// Переписать координаты в последнем снимке.
///
/// Идентификатор и время создания сохраняются: иначе снимок «переезжал» бы в
/// списке при каждом движении окна, и выбрать «как было утром» стало бы
/// невозможно.
pub fn update_last(
    mut snapshots: Vec<Snapshot>,
    sessions: Vec<SnapshotSession>,
    now_s: u64,
) -> Vec<Snapshot> {
    if let Some(first) = snapshots.first_mut() {
        first.sessions = sessions;
        first.updated_s = now_s;
    }
    snapshots
}

/// Человекочитаемый идентификатор: время создания в UTC, без двоеточий.
///
/// UTC, а не местное время, и это осознанно: часового пояса в крейте нет, а
/// тянуть его ради строки, которую человек видит только в жалобах, незачем —
/// час и дату в списке пикер показывает из `created`, форматируя их у себя и в
/// местной зоне.
pub fn snapshot_id(now_s: u64) -> String {
    let days = (now_s / 86_400) as i64;
    let secs = now_s % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}-{:02}-{:02}",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

/// Дата из числа дней с эпохи. Алгоритм Хиннанта — тот же, что в `<chrono>`;
/// взят целиком, чтобы не тянуть крейт ради одной строки в час.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "aaaaaaaa-1111-2222-3333-444444444444";
    const B: &str = "bbbbbbbb-1111-2222-3333-444444444444";

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn rect(x: i32) -> Bounds {
        Bounds { x, y: 0, width: 800, height: 600 }
    }

    fn slots(list: &[(&str, Option<Bounds>)]) -> BTreeMap<String, SlotState> {
        list.iter()
            .map(|(sid, b)| {
                (
                    sid.to_string(),
                    SlotState {
                        bounds: *b,
                        title: "ccfzf".to_string(),
                        cwd: "~/projects/js/ccfzf-picker".to_string(),
                        last_seen_ms: 1_000,
                        focused_at_ms: 0,
                    },
                )
            })
            .collect()
    }

    fn snap(id: &str, created: u64, sessions: Vec<SnapshotSession>) -> Snapshot {
        Snapshot { id: id.to_string(), created_s: created, updated_s: created, sessions }
    }

    fn session(id: &str, x: i32) -> SnapshotSession {
        SnapshotSession {
            id: id.to_string(),
            title: "ccfzf".to_string(),
            cwd: "~/projects/js/ccfzf-picker".to_string(),
            bounds: rect(x),
        }
    }

    #[test]
    fn the_key_does_not_depend_on_the_order() {
        // Порядок сессий задан обходом окон и меняется сам по себе. Ключ,
        // зависящий от него, объявлял бы новый состав на ровном месте.
        assert_eq!(composition_key(&ids(&[A, B])), composition_key(&ids(&[B, A])));
    }

    #[test]
    fn an_empty_composition_is_never_snapshotted() {
        // Закрыл всё на ночь — наутро восстанавливается последний рабочий
        // набор, а не пустота.
        assert_eq!(decide("", "anything", "", 0, 10_000, DEBOUNCE_MS), None);
    }

    #[test]
    fn the_same_composition_only_updates_coordinates() {
        // Окно подвинули, состав тот же. Новой строчки в списке быть не должно,
        // иначе таскание окна мышкой плодило бы снимки, и список стал бы
        // нечитаемым за один день.
        assert_eq!(decide("k", "k", "", 0, 10_000, DEBOUNCE_MS), Some(Decision::Update));
    }

    #[test]
    fn a_new_composition_waits_for_the_debounce() {
        // Пока открываются три сессии подряд, промежуточные конфигурации в
        // историю не попадают.
        assert_eq!(decide("new", "old", "new", 1_000, 1_000 + DEBOUNCE_MS - 1, DEBOUNCE_MS), None);
        assert_eq!(
            decide("new", "old", "new", 1_000, 1_000 + DEBOUNCE_MS, DEBOUNCE_MS),
            Some(Decision::Append)
        );
    }

    #[test]
    fn a_composition_that_only_just_changed_waits_a_full_round() {
        // Ключ ещё не в ожидании — таймер начнётся с этого такта, а решения
        // сейчас нет.
        assert_eq!(decide("new", "old", "другой", 1_000, 999_000, DEBOUNCE_MS), None);
    }

    #[test]
    fn the_timer_restarts_on_every_new_composition() {
        // Снимок фиксирует не момент изменения, а устоявшееся состояние.
        assert_eq!(track_composition("k", "k", 500, 9_000), ("k".to_string(), 500));
        assert_eq!(track_composition("k2", "k", 500, 9_000), ("k2".to_string(), 9_000));
    }

    #[test]
    fn a_session_without_coordinates_does_not_enter_a_snapshot() {
        // Записать её было бы нечестно: восстанавливать нечего, а строка в
        // списке появилась бы.
        let got = sessions_of(&ids(&[A, B]), &slots(&[(A, Some(rect(10))), (B, None)]));
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].id, A);
        assert_eq!(got[0].cwd, "~/projects/js/ccfzf-picker");
    }

    #[test]
    fn snapshot_sessions_are_ordered_by_id() {
        // Порядок обхода окон не наш и меняется. Отпечаток снимка не должен от
        // этого зависеть.
        let got = sessions_of(&ids(&[B, A]), &slots(&[(A, Some(rect(10))), (B, Some(rect(20)))]));
        assert_eq!(got.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), vec![A, B]);
    }

    #[test]
    fn a_new_snapshot_goes_to_the_head_and_the_tail_is_dropped() {
        let old = vec![snap("1", 100, vec![session(A, 0)]), snap("2", 90, vec![session(B, 0)])];
        let got = append(old, "3".to_string(), vec![session(A, 5)], 200, 2);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].id, "3");
        assert_eq!(got[1].id, "1", "вытеснился самый старый");
    }

    #[test]
    fn a_zero_keep_is_read_as_no_setting_not_as_keep_nothing() {
        // Ноль тут — признак незаполненного значения, а не способ выключить
        // снимки числом хранимых: угадывать такой особый случай было бы
        // легче, чем прочитать его в списке.
        let old = vec![snap("1", 100, vec![session(A, 0)]), snap("2", 90, vec![session(B, 0)])];
        let got = append(old, "3".to_string(), vec![session(A, 5)], 200, 0);
        assert_eq!(got.len(), 3, "нулём список не обнулился, взято умолчание KEEP");
    }

    #[test]
    fn updating_the_last_one_keeps_its_id_and_creation_time() {
        // Иначе снимок «переезжал» бы в списке при каждом движении окна, и
        // выбрать «как было утром» стало бы невозможно.
        let old = vec![snap("1", 100, vec![session(A, 0)])];
        let got = update_last(old, vec![session(A, 400)], 300);
        assert_eq!(got[0].id, "1");
        assert_eq!(got[0].created_s, 100);
        assert_eq!(got[0].updated_s, 300);
        assert_eq!(got[0].sessions[0].bounds, rect(400));
    }

    #[test]
    fn updating_an_empty_list_does_nothing() {
        assert!(update_last(Vec::new(), vec![session(A, 0)], 300).is_empty());
    }

    #[test]
    fn the_id_is_readable_and_sorts_by_time() {
        // Идентификатор человек видит: он попадает в тело просьбы о
        // восстановлении и в жалобы. Непрозрачное число здесь стоило бы
        // лишнего шага при каждом разборе.
        assert_eq!(snapshot_id(0), "1970-01-01T00-00-00");
        assert_eq!(snapshot_id(1_765_000_000), "2025-12-06T05-46-40");
        assert!(snapshot_id(1_765_000_000) < snapshot_id(1_765_000_001));
    }
}
