//! Строка, которую человек видит в трее.
//!
//! Трей — единственный канал, где отказ виден без чтения stderr, и места в
//! нём одна строка. Поэтому жалобы не вытесняют друг друга, а приклеиваются к
//! тому, чем трекер занят: отказ расстановки и отказ чтения дампа — про разные
//! машины и разные починки, и одна не должна прятать другую.

/// Чем занят трекер, и что при этом не получилось.
///
/// Пустые заметки опускаются вместе с разделителем: «нечего сказать» и «сказать
/// пустоту» — разные вещи, и вторая оставила бы в строке висящую точку с
/// запятой.
pub fn status_line(base: &str, notes: &[&str]) -> String {
    let mut out = base.to_string();
    for n in notes.iter().filter(|n| !n.is_empty()) {
        out.push_str("; ");
        out.push_str(n);
    }
    out
}

/// Подпись неактивного пункта меню: какая сборка сейчас запущена.
///
/// Нужна затем, что `deploy-mac.sh` обновляет бинарь на месте, а версия у всех
/// сборок между релизами одна: после перезапуска нечем убедиться, что поднялось
/// новое, — приходится ходить за `git log` по ssh.
///
/// Дата опускается у сегодняшней сборки — чаще всего она такая и есть, а
/// повторять сегодняшнее число в трее незачем. «Сегодня» считается от запуска
/// трекера, а не от открытия меню: меню строится один раз при старте, и у
/// процесса, прожившего сутки, подпись покажет вчерашнюю сборку без даты. Цена
/// принята: трекер, проживший сутки, перезапускали не сегодня, и вопрос «то ли
/// собралось» к нему уже не стоит.
///
/// То же правило и тот же формат, что у пикера: два трея на одном экране, и
/// подпись, читаемая по-разному, стоила бы человеку лишнего вопроса.
pub fn version_item_label(
    version: &str,
    built: Option<chrono::NaiveDateTime>,
    today: chrono::NaiveDate,
) -> String {
    let Some(built) = built else {
        return format!("v{version}");
    };
    if built.date() == today {
        format!("v{version} · {}", built.format("%H:%M"))
    } else {
        format!("v{version} · {}", built.format("%Y-%m-%d %H:%M"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, NaiveDateTime};

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> NaiveDateTime {
        day(y, m, d).and_hms_opt(h, min, 0).unwrap()
    }

    #[test]
    fn a_release_is_named_by_its_version_alone() {
        // Ноль в штампе — это «сборка релизная» (см. `build.rs`): её называет
        // версия, а время сборки там лишнее.
        assert_eq!(version_item_label("0.1.0", None, day(2026, 8, 14)), "v0.1.0");
    }

    #[test]
    fn a_build_from_today_shows_the_time_without_the_date() {
        assert_eq!(
            version_item_label("0.1.0", Some(at(2026, 8, 14, 17, 42)), day(2026, 8, 14)),
            "v0.1.0 · 17:42"
        );
    }

    #[test]
    fn an_older_build_keeps_its_date() {
        // Соль пункта: выкатка обновляет бинарь на месте, и вчерашнее время без
        // даты выглядело бы сегодняшним — то есть отвечало бы «перезапустилось
        // новое» на вопрос, ради которого пункт и заведён.
        assert_eq!(
            version_item_label("0.1.0", Some(at(2026, 8, 13, 17, 42)), day(2026, 8, 14)),
            "v0.1.0 · 2026-08-13 17:42"
        );
    }

    #[test]
    fn nothing_to_complain_about_leaves_the_line_alone() {
        assert_eq!(status_line("3 windows tracked", &[]), "3 windows tracked");
        assert_eq!(status_line("3 windows tracked", &["", ""]), "3 windows tracked");
    }

    #[test]
    fn every_complaint_reaches_the_line() {
        // Соль всего модуля: раньше жалоба расстановки жила в той же ячейке,
        // что и итог такта, и успешный итог затирал её в том же обороте. В
        // stderr она оставалась, но трей — единственный канал, где человек
        // видит отказ, не читая логов.
        assert_eq!(
            status_line(
                "3 windows tracked",
                &["place failed: window is gone", "index fetch failed: ssh timeout"]
            ),
            "3 windows tracked; place failed: window is gone; index fetch failed: ssh timeout"
        );
    }

    #[test]
    fn a_single_complaint_needs_no_neighbours() {
        assert_eq!(
            status_line("publish failed: no route", &["place failed: denied", ""]),
            "publish failed: no route; place failed: denied"
        );
    }
}
