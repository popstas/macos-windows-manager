//! Раскладки окон: где чему стоять, когда человек просит разложить.
//!
//! Чистый расчёт: на входе прямоугольник экрана и число окон, на выходе —
//! прямоугольники по порядку. Ни AX, ни номеров окон здесь нет намеренно —
//! тесты этого крейта гоняются на любой машине, а плитка не та вещь, ради
//! которой стоит идти к маку.

use crate::geometry::Bounds;

/// Какой раскладкой раскладывать.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// Сетка без перекрытий.
    Tile,
    /// Стопка со сдвигом: окна одного размера, у каждого виден заголовок.
    Cascade,
}

impl Layout {
    /// Раскладка по имени из просьбы. Имена — те же, что шлёт пикер.
    pub fn from_name(name: &str) -> Option<Layout> {
        match name.trim().to_ascii_lowercase().as_str() {
            "tile" => Some(Layout::Tile),
            "cascade" => Some(Layout::Cascade),
            _ => None,
        }
    }
}

/// Ширина знака моноширинного шрифта в точках.
///
/// Ограничение задано в колонках, а двигаются окна в точках, и перевести одно
/// в другое нечем: Accessibility про шрифт терминала не знает вовсе, а сам
/// терминал своей ширины в знаках не сообщает. Восемь — середина того, что
/// дают ходовые шрифты на маке (Menlo 12 — около 7.2, SF Mono 13 — около 7.8,
/// JetBrains Mono 14 — около 8.4). Число заведомо приблизительное, и это
/// осознанно: ошибка в полточки сдвигает границу на несколько колонок, а не
/// ломает раскладку.
const COL_PT: f64 = 8.0;

/// Что у окна занято не текстом: рамка, отступы, полоса прокрутки.
const CHROME_PT: i32 = 24;

/// Колонок на терминал — не меньше и не больше.
const MIN_COLS: i32 = 80;
const MAX_COLS: i32 = 120;

/// Полоса меню сверху.
///
/// `CGDisplay::bounds()` отдаёт весь экран, а не рабочую область: в нём и
/// полоса меню, и Dock. Рабочую область знает `NSScreen::visibleFrame`, но это
/// API главного потока, а расстановка идёт в потоке трекера — ровно поэтому в
/// `ax::displays()` и выбран `CGDisplay`. Отступ поэтому постоянный. Dock не
/// учитывается вовсе: он бывает снизу, слева и справа, прячется и меняет
/// толщину, и постоянного числа для него не существует.
const MENUBAR_PT: i32 = 25;

/// Наименьшая ширина окна: 80 колонок плюс рамка.
fn min_width() -> i32 {
    (f64::from(MIN_COLS) * COL_PT) as i32 + CHROME_PT
}

/// Наибольшая ширина окна: 120 колонок плюс рамка.
fn max_width() -> i32 {
    (f64::from(MAX_COLS) * COL_PT) as i32 + CHROME_PT
}

/// Рабочая область экрана — всё, кроме полосы меню.
fn work_area(screen: Bounds) -> Bounds {
    Bounds {
        x: screen.x,
        y: screen.y + MENUBAR_PT,
        width: screen.width,
        height: screen.height - MENUBAR_PT,
    }
}

/// Разложить `n` окон по экрану. Порядок ответа — порядок окон.
pub fn arrange(mode: Layout, screen: Bounds, n: usize) -> Vec<Bounds> {
    let work = work_area(screen);
    if n == 0 || work.width <= 0 || work.height <= 0 {
        return Vec::new();
    }
    match mode {
        Layout::Tile => tile(work, n),
        Layout::Cascade => cascade(work, n),
    }
}

/// Сдвиг стопки вниз — высота заголовка окна.
///
/// Меньше нельзя, и это всё содержание каскада: заголовок — единственное, по
/// чему человек выбирает окно из стопки мышью. Накрой соседнее окно заголовок —
/// и стопка превращается в одно верхнее окно с полосками по краю.
const STEP_Y: i32 = 28;

/// Сдвиг стопки вправо. Заголовку не нужен, но без него окна сливаются в одну
/// колонку, и глазом стопка не читается.
const STEP_X: i32 = 36;

/// Стопка со сдвигом вправо и вниз.
///
/// Все окна одного размера — половина экрана по каждой стороне. Кончится место
/// на экране — стопка начинается заново от левого верхнего угла: окон, которым
/// не хватило места на ступеньках, к этому времени полтора десятка, и экрану
/// уже нечего им предложить, как ни считай.
fn cascade(work: Bounds, n: usize) -> Vec<Bounds> {
    let w = work.width / 2;
    let h = work.height / 2;
    // Сколько ступенек помещается: по той стороне, где место кончится раньше.
    let per_stack = 1 + (((work.height - h) / STEP_Y).min((work.width - w) / STEP_X)).max(0);
    (0..n as i32)
        .map(|i| {
            let step = i % per_stack;
            Bounds {
                x: work.x + step * STEP_X,
                y: work.y + step * STEP_Y,
                width: w,
                height: h,
            }
        })
        .collect()
}

/// Сетка без перекрытий.
///
/// Колонок — три, если три дают терминал в пределах 80–120 колонок; шире экран
/// — колонок больше, уже — меньше. Рядов ровно столько, сколько нужно, чтобы
/// разместить все окна: при числе окон больше числа колонок высота делится.
fn tile(work: Bounds, n: usize) -> Vec<Bounds> {
    let cols = columns(work.width, n);
    let rows = div_ceil(n as i32, cols);
    // Ширина режется по 120 колонкам даже там, где экран позволяет больше:
    // растянутый на полтора метра терминал читать нечем — глаз не доносит
    // строку до конца.
    let w = (work.width / cols).min(max_width());
    let h = work.height / rows;
    let mut out = Vec::with_capacity(n);
    for i in 0..n as i32 {
        let (row, col) = (i / cols, i % cols);
        // Ряд центрируется, а не прижимается к левому краю. Ширина обрезана по
        // `max_width`, и неполный ряд — обычное дело: два окна на широком
        // экране жались бы в угол, оставив полтора метра пустоты справа.
        let in_row = (n as i32 - row * cols).min(cols);
        let x0 = work.x + (work.width - in_row * w) / 2;
        out.push(Bounds {
            x: x0 + col * w,
            y: work.y + row * h,
            width: w,
            height: h,
        });
    }
    out
}

/// Сколько колонок в сетке.
///
/// Три — идеал, и от него отступают только под нажимом экрана. Когда экран
/// узок настолько, что оба ограничения разом не выполнить (одна колонка уже
/// шире 120 знаков, а две — уже 80), побеждает нижнее: читать узкий терминал
/// хуже, чем широкий.
fn columns(width: i32, n: usize) -> i32 {
    let most = (width / min_width()).max(1);
    let least = div_ceil(width, max_width()).max(1);
    let cols = 3.max(least).min(most).max(1);
    // Окон меньше, чем колонок, — сетку незачем растягивать впустую: пустая
    // клетка в ряду сдвинула бы центровку и оставила дыру посреди экрана.
    cols.min(n as i32).max(1)
}

/// Деление с округлением вверх. `div_ceil` у целых стабилизирован в 1.73, а
/// заявленный крейтом порог ниже.
fn div_ceil(a: i32, b: i32) -> i32 {
    if b <= 0 {
        return 1;
    }
    (a + b - 1) / b
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Экран ноутбука: 1440×900 в точках.
    const LAPTOP: Bounds = Bounds { x: 0, y: 0, width: 1440, height: 900 };
    /// Широкий внешний монитор: 3440×1440.
    const WIDE: Bounds = Bounds { x: 0, y: 0, width: 3440, height: 1440 };

    #[test]
    fn names_from_the_request_are_understood() {
        assert_eq!(Layout::from_name("tile"), Some(Layout::Tile));
        assert_eq!(Layout::from_name("  TILE  "), Some(Layout::Tile));
        assert_eq!(Layout::from_name("cascade"), Some(Layout::Cascade));
        assert_eq!(Layout::from_name("mosaic"), None);
    }

    #[test]
    fn a_cascade_window_is_half_the_screen() {
        let got = arrange(Layout::Cascade, WIDE, 3);
        let work_h = WIDE.height - MENUBAR_PT;
        assert!(got.iter().all(|b| b.width == WIDE.width / 2 && b.height == work_h / 2), "{got:?}");
    }

    #[test]
    fn every_cascade_window_shows_its_title() {
        // Соль каскада: заголовок — единственное, по чему человек выбирает окно
        // из стопки мышью. Сдвиг меньше высоты заголовка накрыл бы его соседним
        // окном, и стопка стала бы одним верхним окном с полосками по краю.
        let got = arrange(Layout::Cascade, WIDE, 5);
        for pair in got.windows(2) {
            assert!(pair[1].y - pair[0].y >= STEP_Y, "{:?} накрывает заголовок {:?}", pair[1], pair[0]);
            assert!(pair[1].x > pair[0].x, "стопка сдвигается и вправо: {pair:?}");
        }
    }

    #[test]
    fn the_cascade_starts_below_the_menu_bar() {
        let got = arrange(Layout::Cascade, WIDE, 1);
        assert_eq!(got[0].x, WIDE.x);
        assert_eq!(got[0].y, MENUBAR_PT);
    }

    #[test]
    fn no_cascade_window_hangs_off_the_screen() {
        // Ступеньки уводят окна вправо и вниз, и без счёта места последнее
        // уехало бы за край — туда, где мышью его не достать.
        for n in 1..=40 {
            for screen in [LAPTOP, WIDE] {
                let work_bottom = screen.y + screen.height;
                for b in arrange(Layout::Cascade, screen, n) {
                    assert!(b.x + b.width <= screen.x + screen.width, "{n}: {b:?}");
                    assert!(b.y + b.height <= work_bottom, "{n}: {b:?}");
                }
            }
        }
    }

    #[test]
    fn a_stack_that_ran_out_of_room_begins_anew() {
        // Экран, на котором ступенек помещается всего семь. Восьмое окно
        // начинает стопку заново от угла: место на ступеньках кончилось, и
        // предложить ему экрану больше нечего.
        let small = Bounds { x: 0, y: 0, width: 1000, height: 400 };
        let got = arrange(Layout::Cascade, small, 9);
        assert_eq!(got[7], got[0], "восьмое окно встало на место первого");
        assert_eq!(got[8], got[1]);
    }

    #[test]
    fn a_second_screen_holds_the_cascade_too() {
        let right = Bounds { x: 1440, y: 0, width: 1440, height: 900 };
        let got = arrange(Layout::Cascade, right, 4);
        assert!(got.iter().all(|b| b.x >= right.x), "{got:?}");
    }

    #[test]
    fn nothing_to_place_is_not_a_layout() {
        assert!(arrange(Layout::Tile, WIDE, 0).is_empty());
    }

    #[test]
    fn the_menu_bar_is_not_part_of_the_screen() {
        // `CGDisplay::bounds()` отдаёт весь экран, полосу меню включительно.
        // Разложи мы по нему — верхний ряд ушёл бы под меню, и заголовок
        // первого окна человек не увидел бы вовсе.
        let got = arrange(Layout::Tile, WIDE, 1);
        assert_eq!(got[0].y, MENUBAR_PT);
        assert_eq!(got[0].height, WIDE.height - MENUBAR_PT);
    }

    #[test]
    fn three_windows_stand_in_one_row() {
        // Идеал из задачи: три вертикальных терминала на экран.
        let got = arrange(Layout::Tile, WIDE, 3);
        assert_eq!(got.len(), 3);
        let ys: Vec<i32> = got.iter().map(|b| b.y).collect();
        assert_eq!(ys, vec![MENUBAR_PT; 3], "один ряд — одна высота");
        assert!(got[0].x < got[1].x && got[1].x < got[2].x, "порядок слева направо");
        assert_eq!(got[0].height, WIDE.height - MENUBAR_PT, "ряд один — высота полная");
    }

    #[test]
    fn no_window_is_narrower_than_eighty_columns() {
        // Соль ограничения: терминал уже 80 знаков ломает вывод любой команды,
        // которая рисует таблицу.
        for n in 1..=8 {
            for screen in [LAPTOP, WIDE] {
                for b in arrange(Layout::Tile, screen, n) {
                    assert!(
                        b.width >= min_width(),
                        "{n} окон на {}: ширина {} меньше 80 колонок",
                        screen.width,
                        b.width
                    );
                }
            }
        }
    }

    #[test]
    fn no_window_is_wider_than_a_hundred_and_twenty_columns() {
        for n in 1..=8 {
            for screen in [LAPTOP, WIDE] {
                for b in arrange(Layout::Tile, screen, n) {
                    assert!(
                        b.width <= max_width(),
                        "{n} окон на {}: ширина {} больше 120 колонок",
                        screen.width,
                        b.width
                    );
                }
            }
        }
    }

    #[test]
    fn a_narrow_screen_takes_fewer_columns_than_three() {
        // 1440 на три — по 480 точек, это 57 колонок. Три сюда не влезают, и
        // правило «не меньше 80» важнее идеала «три в ряд».
        let got = arrange(Layout::Tile, LAPTOP, 3);
        let in_top_row = got.iter().filter(|b| b.y == MENUBAR_PT).count();
        assert_eq!(in_top_row, 2, "на ноутбуке в ряд встают двое");
        assert_eq!(got.len(), 3);
        assert!(got[2].y > got[0].y, "третье окно ушло во второй ряд");
    }

    #[test]
    fn a_wide_screen_takes_more_columns_than_three() {
        // 3440 на три — по 1146 точек, это 140 колонок. Строку такой ширины
        // глаз не доносит до конца, поэтому колонок становится четыре.
        let got = arrange(Layout::Tile, WIDE, 4);
        let in_top_row = got.iter().filter(|b| b.y == MENUBAR_PT).count();
        assert_eq!(in_top_row, 4, "на широком экране в ряд встают четверо");
    }

    #[test]
    fn more_windows_than_columns_split_the_height() {
        // Ровно то, что просили: «если больше, то высоту делить на 2».
        let got = arrange(Layout::Tile, WIDE, 8);
        let rows: std::collections::BTreeSet<i32> = got.iter().map(|b| b.y).collect();
        assert_eq!(rows.len(), 2, "восемь окон при четырёх колонках — два ряда");
        let h = (WIDE.height - MENUBAR_PT) / 2;
        assert!(got.iter().all(|b| b.height == h), "высота поделена поровну");
    }

    #[test]
    fn windows_in_a_row_do_not_overlap() {
        // Плитка тем и отличается от каскада: перекрытий нет.
        for n in 1..=8 {
            let got = arrange(Layout::Tile, WIDE, n);
            for pair in got.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                if a.y != b.y {
                    continue;
                }
                assert!(a.x + a.width <= b.x, "{n} окон: {a:?} лезет на {b:?}");
            }
        }
    }

    #[test]
    fn a_short_row_stands_in_the_middle() {
        // Два окна на широком экране: ширина обрезана по 120 колонкам, и без
        // центровки они прижались бы к левому краю, оставив пустоту справа.
        let got = arrange(Layout::Tile, WIDE, 2);
        let left = got[0].x;
        let right = WIDE.width - (got[1].x + got[1].width);
        assert!((left - right).abs() <= 1, "поля слева {left} и справа {right} равны");
        assert!(left > 0, "поля вообще есть — иначе центровки не было");
    }

    #[test]
    fn the_last_row_is_centred_too() {
        // Пять окон при четырёх колонках: во втором ряду одно, и стоять ему
        // посреди экрана, а не в левом углу под первым.
        let got = arrange(Layout::Tile, WIDE, 5);
        let last = got[4];
        let left = last.x;
        let right = WIDE.width - (last.x + last.width);
        assert!((left - right).abs() <= 1, "одинокое окно ряда стоит посередине");
    }

    #[test]
    fn a_second_screen_gets_its_own_coordinates() {
        // Экран не обязан начинаться в нуле: у второго монитора начало — там,
        // где кончился первый. Раскладка, забывшая про это, разложила бы окна
        // на чужом экране.
        let right = Bounds { x: 1440, y: 0, width: 1440, height: 900 };
        let got = arrange(Layout::Tile, right, 2);
        assert!(got.iter().all(|b| b.x >= right.x), "{got:?}");
        assert!(got.iter().all(|b| b.x + b.width <= right.x + right.width), "{got:?}");
    }

    #[test]
    fn a_screen_too_narrow_for_the_rule_keeps_one_column() {
        // Экран, на котором одна колонка уже шире 120 знаков, а две — уже 80.
        // Оба ограничения разом не выполнить; побеждает нижнее, и окно
        // остаётся одно на ряд, шире обещанного.
        let narrow = Bounds { x: 0, y: 0, width: 1000, height: 800 };
        let got = arrange(Layout::Tile, narrow, 2);
        assert_eq!(got[0].y + got[0].height, got[1].y, "оба окна в своём ряду");
        assert!(got[0].width >= min_width(), "ширина не опустилась ниже 80 колонок");
    }
}
