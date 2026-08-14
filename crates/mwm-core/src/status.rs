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

#[cfg(test)]
mod tests {
    use super::*;

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
