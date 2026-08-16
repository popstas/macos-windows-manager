//! Заголовок окна в том виде, в котором его сравнивают с заголовком сессии.

/// Снять со значка состояния и привести к сравнимому виду.
///
/// Значок опознаётся по разряду символа (`char::is_symbol`-эквивалент), а не по
/// конкретному знаку: это индикатор, и он меняется от версии к версии Claude
/// Code. Знаки препинания не снимаются никогда — заголовок, начинающийся с
/// тире или кавычки, законен, и его правка развела бы две стороны сравнения.
///
/// Снимается и приставка терминала — она стоит **перед** значком, потому что
/// значок принадлежит заголовку панели, а приставку дописывает вокруг него сам
/// терминал (см. `strip_window_prefix`).
pub fn strip_decoration(title: &str) -> String {
    let rest = strip_window_prefix(title.trim_start());
    let head: String = rest.chars().take_while(|c| is_symbol(*c)).collect();
    let out = if head.is_empty() {
        rest
    } else {
        let tail = &rest[head.len()..];
        // Значок отделён от заголовка пробелом. Без пробела это не значок, а
        // первый символ самого заголовка.
        if tail.starts_with(char::is_whitespace) { tail } else { rest }
    };
    out.trim().to_string()
}

/// Снять приставку, которую дописывает к заголовку окна сам терминал.
///
/// Такая приставка есть у WezTerm: умолчание `format-window-title` собирает
/// заголовок окна как `[Z] [i/n] <заголовок панели>` — `[Z]` у развёрнутой
/// панели, `[i/n]` при двух и более вкладках. Заголовок панели там же, где у
/// остальных терминалов, но дамп хранит голое имя сессии, и сравнение по
/// `strip_decoration` не сходится — окно не привязывается к сессии вовсе.
/// Отказ молчащий: непривязанное окно просто не попадает в файл трекера.
///
/// Снимаются ровно две эти формы, а не всякая скобка в начале: `[wip] fix`
/// — законное имя сессии, и его правка развела бы две стороны сравнения там,
/// где они сходились. По той же причине обязателен пробел после скобки —
/// без него это первый символ самого заголовка, а не приставка.
fn strip_window_prefix(title: &str) -> &str {
    let mut rest = title;
    loop {
        let Some(tail) = rest.strip_prefix('[') else { return rest };
        let Some((inside, after)) = tail.split_once(']') else { return rest };
        if !is_wezterm_prefix(inside) {
            return rest;
        }
        let Some(after) = after.strip_prefix(char::is_whitespace) else { return rest };
        rest = after.trim_start();
    }
}

/// Содержимое скобки, которую дописывает WezTerm: `Z` или `i/n` из цифр.
fn is_wezterm_prefix(inside: &str) -> bool {
    if inside == "Z" {
        return true;
    }
    match inside.split_once('/') {
        Some((a, b)) => {
            !a.is_empty()
                && !b.is_empty()
                && a.chars().all(|c| c.is_ascii_digit())
                && b.chars().all(|c| c.is_ascii_digit())
        }
        None => false,
    }
}

/// Значок ли это — то есть не буква, не цифра, не пробел и не препинание.
///
/// Своя проверка, а не таблица разрядов Unicode из крейта: нужны ровно четыре
/// разряда, и полная таблица ради них в дерево сборки не поедет. Приближение
/// честное — всё, что эта проверка пропустит вперёд, и есть значок состояния,
/// который надо снять.
fn is_symbol(c: char) -> bool {
    !c.is_alphanumeric() && !c.is_whitespace() && !is_punctuation(c)
}

/// Препинание: ASCII плюс те немногие не-ASCII знаки, что встречаются в
/// заголовках сессий. Их снимать нельзя — заголовок, честно начинающийся с
/// тире или кавычки, обязан дожить до сравнения целым.
fn is_punctuation(c: char) -> bool {
    c.is_ascii_punctuation()
        || matches!(c, '–' | '—' | '«' | '»' | '\u{201C}' | '\u{201D}' | '\u{2018}' | '\u{2019}' | '…')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyph_prefix_is_stripped() {
        // Claude Code ставит перед заголовком значок состояния, пока работает,
        // а дамп хранит голую сводку. Без снятия значка две стороны сравнения
        // перестают сходиться ровно тогда, когда сессия работает.
        assert_eq!(strip_decoration("✳ Check branch commit count"), "Check branch commit count");
    }

    #[test]
    fn punctuation_survives() {
        // Снимаются только символы, но не знаки препинания: заголовок, честно
        // начинающийся с тире или кавычки, обязан дожить до сравнения целым.
        assert_eq!(strip_decoration("- fix the parser"), "- fix the parser");
        assert_eq!(strip_decoration("\"quoted\" title"), "\"quoted\" title");

        // Типографские кавычки и тире: это единственное, что защищает их от
        // классификации как символы и снятия. ASCII-пунктуация защищена другой
        // проверкой, так что тест без этого блока ничего про matches! не докажет.
        // Каждый символ ниже есть в matches! — если вынуть его оттуда, тест упадёт.
        assert_eq!(strip_decoration("– en dash title"), "– en dash title");
        assert_eq!(strip_decoration("— em dash title"), "— em dash title");
        assert_eq!(strip_decoration("« guillemet left"), "« guillemet left");
        assert_eq!(strip_decoration("» guillemet right"), "» guillemet right");
        assert_eq!(strip_decoration("\u{201C} curly double left"), "\u{201C} curly double left");
        assert_eq!(strip_decoration("\u{201D} curly double right"), "\u{201D} curly double right");
        assert_eq!(strip_decoration("\u{2018} curly single left"), "\u{2018} curly single left");
        assert_eq!(strip_decoration("\u{2019} curly single right"), "\u{2019} curly single right");
        assert_eq!(strip_decoration("… ellipsis title"), "… ellipsis title");
    }

    #[test]
    fn wezterm_tab_index_is_stripped() {
        // WezTerm по умолчанию дописывает в заголовок окна номер вкладки —
        // `[%d/%d] ` при двух и более вкладках и `[Z] ` у развёрнутой панели
        // (`format-window-title`). Дамп хранит голое имя сессии, и без снятия
        // этой приставки окно не привязывается вовсе — ровно тогда, когда
        // человек открыл в терминале вторую вкладку.
        assert_eq!(strip_decoration("[1/2] windows-build-speed"), "windows-build-speed");
        assert_eq!(strip_decoration("[Z] windows-build-speed"), "windows-build-speed");
        assert_eq!(strip_decoration("[Z] [10/12] windows-build-speed"), "windows-build-speed");
        // Значок состояния Claude Code стоит внутри заголовка панели, то есть
        // после приставки: снимаются оба, и порядок снятия обязан быть этот.
        assert_eq!(strip_decoration("[1/2] ✳ windows-build-speed"), "windows-build-speed");
    }

    #[test]
    fn other_bracket_prefixes_survive() {
        // Снимается не всякая скобка, а ровно две формы WezTerm. Заголовок
        // сессии, честно начинающийся со скобки, — законное имя, и снятие
        // развело бы две стороны сравнения на пустом месте.
        assert_eq!(strip_decoration("[wip] fix parser"), "[wip] fix parser");
        assert_eq!(strip_decoration("[2] second try"), "[2] second try");
        assert_eq!(strip_decoration("[1/2]no space"), "[1/2]no space");
    }

    #[test]
    fn edges_are_trimmed_and_empty_survives() {
        assert_eq!(strip_decoration("  ccfzf  "), "ccfzf");
        assert_eq!(strip_decoration(""), "");
        assert_eq!(strip_decoration("✳   "), "");
    }
}
