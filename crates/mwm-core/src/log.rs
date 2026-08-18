//! Строки лога: в stderr и в кольцевой буфер разом.
//!
//! Запущенное из `.app` приложение stderr отдаёт некуда — launchd его никуда
//! не кладёт, а из трея консоли не видно. На вопрос «почему трекер притих»
//! смотреть было не во что: ни файла, ни буфера. Буфер и заведён затем, чтобы
//! вкладка Log в окне настроек показывала то же, что раньше уходило в никуда.
//!
//! Буфер один на процесс, а не в состоянии приложения: писать в него нужно и
//! из `src-tauri`, и отсюда, из `mwm-core`, а до состояния Tauri этот крейт не
//! дотягивается и дотягиваться не должен — он не знает про Tauri вовсе.

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

/// Сколько строк держим.
///
/// Тысяча — это примерно сутки жизни трекера: печатается он на смену картины,
/// а не каждый такт (см. `diag::binding_note`). Мерка выбрана так, чтобы
/// вчерашняя поломка ещё лежала в буфере, когда про неё спросят утром.
const CAPACITY: usize = 1000;

/// Кольцо строк: новая вытесняет самую старую.
///
/// Отдельным типом, а не парой функций над глобальной ячейкой, ровно ради
/// тестов: глобальную ячейку тесты делили бы между собой, и вытеснение
/// проверялось бы на буфере, куда успел написать сосед.
pub struct Ring {
    lines: VecDeque<String>,
    capacity: usize,
}

impl Ring {
    pub fn new(capacity: usize) -> Self {
        Self { lines: VecDeque::with_capacity(capacity), capacity }
    }

    pub fn push(&mut self, line: String) {
        if self.lines.len() == self.capacity {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
    }

    /// Снимок, от самой старой строки к самой новой.
    pub fn lines(&self) -> Vec<String> {
        self.lines.iter().cloned().collect()
    }
}

/// Строка лога целиком: время, имя приложения, сообщение.
///
/// Время ставится всем строкам без исключения. Раньше его несла ровно одна
/// (жалоба на непривязанные окна), и довод там был верен для всех: launchd
/// отметок не ставит, а жалоба нужна затем, чтобы лечь рядом с событием на
/// другой машине — сном, разрывом, перезапуском. Без времени сводить её не с
/// чем.
///
/// Дата, а не одно время: трекер живёт неделями, и «14:07» без числа отвечает
/// на вопрос «когда» только в первые сутки.
///
/// Отдельной функцией — чтобы формат проверялся тестом, а не глазами по
/// двадцати местам вызова.
pub fn stamped(now: chrono::NaiveDateTime, message: &str) -> String {
    format!("{} mwm: {message}", now.format("%Y-%m-%d %H:%M:%S"))
}

fn ring() -> &'static Mutex<Ring> {
    static RING: OnceLock<Mutex<Ring>> = OnceLock::new();
    RING.get_or_init(|| Mutex::new(Ring::new(CAPACITY)))
}

/// Записать строку: в stderr и в буфер разом.
///
/// stderr остаётся на месте и после появления буфера: запущенный из терминала
/// трекер читают именно там, и отладка «запусти и смотри» дешевле открывания
/// окна настроек.
pub fn write(message: &str) {
    let line = stamped(chrono::Local::now().naive_local(), message);
    eprintln!("{line}");
    // Отравленный мьютекс не повод терять строку: паника в чужом потоке — как
    // раз тот случай, ради которого лог и читают.
    let mut ring = ring().lock().unwrap_or_else(|e| e.into_inner());
    ring.push(line);
}

/// Снимок буфера для вкладки Log.
pub fn lines() -> Vec<String> {
    ring().lock().unwrap_or_else(|e| e.into_inner()).lines()
}

/// Строка лога: `mwm_log!("cannot read {path}: {e}")`.
///
/// Имя приложения и время подставляет `write`, а не место вызова: двадцать
/// мест, каждое со своим `mwm: `, рано или поздно разошлись бы в написании, а
/// вкладка Log читается глазами по левому краю.
#[macro_export]
macro_rules! mwm_log {
    ($($arg:tt)*) => {
        $crate::log::write(&format!($($arg)*))
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_oldest_line_leaves_when_the_ring_is_full() {
        let mut ring = Ring::new(3);
        for n in 1..=4 {
            ring.push(format!("line {n}"));
        }
        assert_eq!(ring.lines(), vec!["line 2", "line 3", "line 4"]);
    }

    #[test]
    fn a_ring_with_room_to_spare_keeps_everything_in_order() {
        let mut ring = Ring::new(3);
        ring.push("first".to_string());
        ring.push("second".to_string());
        // Порядок — от старой к новой: вкладка Log дописывает снизу, как это
        // делает любой лог, и перевёрнутый список читался бы задом наперёд.
        assert_eq!(ring.lines(), vec!["first", "second"]);
    }

    #[test]
    fn every_line_carries_the_date_and_the_name() {
        // Дата в строке — не украшение: жалобу сводят с событием на другой
        // машине, и «14:07» без числа отвечает на «когда» только сегодня.
        let now = chrono::NaiveDate::from_ymd_opt(2026, 8, 18)
            .unwrap()
            .and_hms_opt(14, 7, 3)
            .unwrap();
        assert_eq!(
            stamped(now, "cannot read the dump"),
            "2026-08-18 14:07:03 mwm: cannot read the dump"
        );
    }
}
