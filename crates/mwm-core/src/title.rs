//! Заголовок окна в том виде, в котором его сравнивают с заголовком сессии.

/// Снять со значка состояния и привести к сравнимому виду.
///
/// Значок опознаётся по разряду символа (`char::is_symbol`-эквивалент), а не по
/// конкретному знаку: это индикатор, и он меняется от версии к версии Claude
/// Code. Знаки препинания не снимаются никогда — заголовок, начинающийся с
/// тире или кавычки, законен, и его правка развела бы две стороны сравнения.
pub fn strip_decoration(title: &str) -> String {
    let rest = title.trim_start();
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
        || matches!(c, '–' | '—' | '«' | '»' | '"' | '"' | '\u{2018}' | '\u{2019}' | '…')
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
    }

    #[test]
    fn edges_are_trimmed_and_empty_survives() {
        assert_eq!(strip_decoration("  ccfzf  "), "ccfzf");
        assert_eq!(strip_decoration(""), "");
        assert_eq!(strip_decoration("✳   "), "");
    }
}
