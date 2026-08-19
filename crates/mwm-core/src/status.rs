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

/// Чем занят трекер в обычном такте: сколько окон ведётся и сколько из них
/// свёрнуто.
///
/// Свёрнутые считаются среди ведомых, а не среди всех виденных: пункты меню
/// ниже строки — это ведомые окна, и число, посчитанное по другому множеству,
/// говорило бы не про тот список, который человек под ним видит.
///
/// Ноль свёрнутых опускается вместе с запятой — по тому же правилу, что и
/// пустая заметка в `status_line`: «свёрнутых нет» и «сказать про ноль» разные
/// вещи, а вторая приписывала бы к строке хвост, который висел бы там всегда.
/// Хвост не зависит от того, включён ли пропуск свёрнутых: человек, увидевший
/// в плитке на одно окно меньше, спрашивает «где оно», и ответ обязан стоять в
/// строке до того, как он полезет в настройки.
pub fn tracked_line(tracked: usize, minimized: usize) -> String {
    if minimized == 0 {
        format!("{tracked} windows tracked")
    } else {
        format!("{tracked} windows tracked, {minimized} minimized")
    }
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

/// Сколько знаков заголовка помещается в пункт меню.
///
/// Заголовок сессии длины не обещает, а меню трея растягивается по самому
/// длинному пункту: одно окно с длинным именем растянуло бы его на весь экран,
/// и остальные пункты уехали бы человеку под курсор мышью через пол-экрана.
const TITLE_LIMIT: usize = 60;

/// Подпись пункта меню про одно ведомое окно.
///
/// Терминал приписывается затем, что заголовок сессии сам по себе не говорит,
/// где она открыта, а окна одной сессии человек ищет именно по терминалу.
/// Пустое имя терминала опускается вместе с разделителем — по тому же правилу,
/// что и пустая заметка в `status_line`: висящая точка посреди пункта говорила
/// бы о терминале, которого нет.
pub fn window_item_label(title: &str, app: &str) -> String {
    let title = ellipsize(title.trim(), TITLE_LIMIT);
    let app = app.trim();
    if app.is_empty() {
        title
    } else {
        format!("{title} \u{b7} {app}")
    }
}

/// Обрезка по знакам, а не по байтам: заголовок сессии бывает и кириллицей, а
/// срез по байтам посреди буквы — это паника, а не длинный пункт меню.
fn ellipsize(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let head: String = text.chars().take(limit - 1).collect();
    format!("{}\u{2026}", head.trim_end())
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
    fn a_tick_without_minimized_windows_says_nothing_about_them() {
        assert_eq!(tracked_line(3, 0), "3 windows tracked");
    }

    #[test]
    fn minimized_windows_are_counted_after_the_tracked_ones() {
        assert_eq!(tracked_line(3, 1), "3 windows tracked, 1 minimized");
    }

    /// Свёрнуты все — строка всё равно называет оба числа: «3 windows tracked»
    /// с пропавшей плиткой выглядело бы поломкой раскладки, а не свёрнутыми
    /// окнами.
    #[test]
    fn all_windows_minimized_still_names_both_numbers() {
        assert_eq!(tracked_line(3, 3), "3 windows tracked, 3 minimized");
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
    fn a_window_is_named_by_its_title_and_its_terminal() {
        // Соль пункта: заголовок сессии не говорит, где она открыта, а окна
        // одной сессии человек ищет именно по терминалу.
        assert_eq!(window_item_label("claude-wt", "WezTerm"), "claude-wt \u{b7} WezTerm");
    }

    #[test]
    fn an_unnamed_terminal_takes_the_separator_with_it() {
        // То же правило, что у пустой заметки в `status_line`: висящая точка
        // посреди пункта говорила бы о терминале, которого нет.
        assert_eq!(window_item_label("claude-wt", ""), "claude-wt");
        assert_eq!(window_item_label("  claude-wt  ", "   "), "claude-wt");
    }

    #[test]
    fn a_long_title_is_cut_and_its_terminal_survives_the_cut() {
        // Меню трея растягивается по самому длинному пункту, и одно окно с
        // длинным именем уводило бы остальные пункты человеку под курсор
        // мышью через пол-экрана. Терминал режется вместе с заголовком —
        // тогда пункт перестал бы отвечать на вопрос, ради которого заведён.
        let label = window_item_label(&"a".repeat(200), "WezTerm");
        assert!(label.ends_with("\u{2026} \u{b7} WezTerm"), "{label}");
        let title = label.split(" \u{b7} ").next().unwrap();
        assert_eq!(title.chars().count(), TITLE_LIMIT, "{label}");
    }

    #[test]
    fn a_title_that_fits_is_left_alone() {
        let title = "a".repeat(TITLE_LIMIT);
        assert_eq!(window_item_label(&title, ""), title);
    }

    #[test]
    fn cutting_counts_letters_not_bytes() {
        // Срез по байтам посреди кириллической буквы — это паника, а не
        // длинный пункт меню.
        let label = window_item_label(&"\u{44f}".repeat(200), "");
        assert_eq!(label.chars().count(), TITLE_LIMIT);
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
