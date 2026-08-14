//! Жалоба на окно, которое трекер видит, но никому не отдал.
//!
//! До неё в stderr попадали только отказ расстановки и ошибка чтения дампа, а
//! самая частая поломка — живое окно пропало из публикуемого списка — не
//! попадала никуда: файл писался исправно, просто пустым, и «окон нет вовсе»
//! выглядело неотличимо от «окна есть, но их некому отдать». Проверено на
//! живой машине дважды за день (15:07 и 18:49): окна на экране, сессии живы,
//! в списке ноль, в логе ни слова.

/// Что видно на экране и что из этого досталось сессиям.
///
/// `None` — сказать нечего: каждое видимое окно привязано. Молчание здесь и
/// есть норма, поэтому строка не печатается «для полноты»: печатать её каждый
/// такт значило бы утопить в ней те такты, где она что-то значит.
///
/// Поводом считается только непривязанное окно, но не `unresolved` сам по
/// себе. Заголовок попадает в `unresolved` и у окна, привязка которого цела
/// (в том же терминале запустили claude заново — id новый, окно прежнее), и
/// такой такт — норма, а не поломка. Печатать по нему значило бы жаловаться на
/// каждую работающую сессию: у неё заголовок меняется с каждым ответом агента.
///
/// `unresolved` при этом едет в строку как объяснение: непривязанное окно с
/// заголовком, которого дамп не знает, и непривязанное окно с заголовком,
/// который дамп знает прекрасно, — две разные поломки с разными починками, и
/// различает их ровно этот список.
///
/// Заголовки печатаются как есть, со значком состояния и в кавычках: сравнение
/// не сходится обычно как раз на невидимом — на хвостовом пробеле, на значке,
/// который не сняли, на пустой строке вместо имени.
pub fn binding_note(
    seen: usize,
    bound: usize,
    unbound: &[String],
    unresolved: &[String],
) -> Option<String> {
    if unbound.is_empty() {
        return None;
    }
    let quoted = |v: &[String]| {
        v.iter().map(|s| format!("{s:?}")).collect::<Vec<_>>().join(", ")
    };
    let mut out = format!("seen {seen} / bound {bound}; unbound: {}", quoted(unbound));
    if !unresolved.is_empty() {
        out.push_str(&format!("; unresolved: {}", quoted(unresolved)));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn every_window_bound_means_nothing_to_say() {
        assert_eq!(binding_note(2, 2, &[], &[]), None);
        // Тот же ответ и когда дамп чего-то не знает: пока все окна привязаны,
        // это не поломка, а обычный такт работающей сессии.
        assert_eq!(binding_note(2, 2, &[], &v(&["away-plan"])), None);
    }

    #[test]
    fn an_unbound_window_is_named() {
        // Ради этого случая модуль и заведён: окна на экране есть, в списке
        // ноль, и до сих пор об этом нельзя было узнать ниоткуда.
        assert_eq!(
            binding_note(2, 0, &v(&["✳ away-plan", "hide-grant-when-trusted"]), &[]),
            Some(
                r#"seen 2 / bound 0; unbound: "✳ away-plan", "hide-grant-when-trusted""#
                    .to_string()
            )
        );
    }

    #[test]
    fn the_dump_is_blamed_only_when_it_is_to_blame() {
        // Две разные поломки, и различает их `unresolved`. Заголовок в нём —
        // дамп про такую сессию не знает: чинить на той стороне. Заголовка
        // нет — дамп знает, а привязка всё равно не случилась: чинить здесь.
        assert_eq!(
            binding_note(1, 0, &v(&["away-plan"]), &v(&["away-plan"])),
            Some(r#"seen 1 / bound 0; unbound: "away-plan"; unresolved: "away-plan""#.to_string())
        );
        assert_eq!(
            binding_note(1, 0, &v(&["away-plan"]), &[]),
            Some(r#"seen 1 / bound 0; unbound: "away-plan""#.to_string())
        );
    }

    #[test]
    fn invisible_differences_survive_into_the_line() {
        // Сравнение заголовков не сходится обычно на том, чего не видно.
        // Кавычки и escape в `{:?}` — единственное, что отличает "away-plan"
        // от "away-plan " в логе.
        assert_eq!(
            binding_note(1, 0, &v(&["away-plan "]), &[]),
            Some(r#"seen 1 / bound 0; unbound: "away-plan ""#.to_string())
        );
    }

    #[test]
    fn a_lost_window_is_reported_even_when_its_neighbour_is_fine() {
        // Частичная потеря — тоже потеря: на панели пропадает одна строка из
        // двух, и по счётчикам это видно сразу.
        assert_eq!(
            binding_note(2, 1, &v(&["away-plan"]), &[]),
            Some(r#"seen 2 / bound 1; unbound: "away-plan""#.to_string())
        );
    }
}
