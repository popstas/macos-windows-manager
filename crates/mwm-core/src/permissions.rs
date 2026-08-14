//! Что сказать человеку, когда разрешения нет.
//!
//! Текст живёт отдельным модулем, а не строкой у места вызова, по одной
//! причине: в нём есть путь, и путь этот обязан совпадать с тем, который
//! человек выберет в диалоге macOS. Ошибиться в нём легко, а заметить ошибку
//! нельзя ничем, кроме повторения всей истории с разрешениями. Здесь его
//! сторожит тест.

use std::path::Path;

/// Жалоба на отсутствующее разрешение Accessibility.
///
/// Без него трекер не видит ни одного окна и не публикует ничего — и это
/// единственная ветка, где он молчал бы, не назвав причины: список окон пуст,
/// ошибок нет, процесс жив. Отсюда и подробность жалобы: она заменяет собой
/// расследование, которое иначе начинается с вопроса «а он вообще работает».
///
/// Путь называется полный и именно тот, по которому запущен этот процесс.
/// Диалог macOS показывает файлы, а не программы, и «добавьте трекер» человеку
/// не помогает: у сборки путь длинный, лежит она не там, где ищут, а
/// `~/projects/...` в диалоге не набрать.
///
/// Отдельной строкой — про повторное добавление. Требование к подписи у сборки
/// без сертификата содержит хеш содержимого, поэтому пересобранный бинарь для
/// macOS — другая программа: старая запись в списке остаётся с галкой и не
/// значит ничего. Человек в этот момент видит включённое разрешение и молчащий
/// трекер, и без подсказки идёт искать ошибку в коде.
///
/// `None` значит «путь узнать не удалось» — тогда честнее сказать об этом, чем
/// назвать выдуманный: неверный путь в такой подсказке дороже отсутствующего.
pub fn accessibility_missing(exe: Option<&Path>) -> String {
    let file = match exe {
        Some(p) => p.display().to_string(),
        None => "(the path of this binary could not be determined)".to_string(),
    };
    format!(
        "Accessibility is not granted: no windows are visible and nothing is published.\n\
         Add this exact file in System Settings > Privacy & Security > Accessibility:\n\
         \x20   {file}\n\
         If it is already in the list, remove it and add it again: a rebuilt binary\n\
         is a different program to macOS, and the old entry keeps its checkmark\n\
         while granting nothing."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn the_message_names_the_running_binary() {
        // Ради этого пути жалоба и написана: диалог macOS показывает файлы, и
        // человеку нужен тот самый, а не название программы.
        let p = PathBuf::from("/Users/user/projects/macos-windows-manager/target/release/macos-windows-manager");
        let msg = accessibility_missing(Some(&p));
        assert!(
            msg.contains("/target/release/macos-windows-manager"),
            "путь обязан быть в жалобе целиком, иначе она не заменяет расследование"
        );
        assert!(msg.contains("Accessibility"), "жалоба обязана называть само разрешение");
    }

    #[test]
    fn the_message_tells_to_re_add_a_stale_entry() {
        // Самая дорогая часть подсказки: запись от прежней сборки остаётся в
        // списке с галкой и не значит ничего. Без этой строки человек видит
        // включённое разрешение и молчащий трекер — и идёт искать ошибку в
        // коде, а не в списке.
        let msg = accessibility_missing(Some(&PathBuf::from("/tmp/x")));
        assert!(
            msg.contains("remove it and add it again"),
            "подсказка про повторное добавление обязана быть — на ней держится вся её польза"
        );
    }

    #[test]
    fn an_unknown_path_is_admitted_not_invented() {
        // Неверный путь в такой подсказке дороже отсутствующего: человек
        // добавит не тот файл и будет уверен, что разрешение выдал.
        let msg = accessibility_missing(None);
        assert!(msg.contains("could not be determined"), "о незнании пути говорят прямо");
        assert!(
            !msg.contains("target/release"),
            "выдуманного пути в жалобе быть не должно"
        );
    }
}
