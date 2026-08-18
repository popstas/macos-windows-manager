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

/// Полоса меню сверху — только для запасной рабочей области.
///
/// Настоящую отдаёт `NSScreen::visibleFrame`, и в ней вычтен и Dock, с какой бы
/// стороны он ни стоял. Но это API главного потока, а расстановка идёт в потоке
/// трекера, и в тот такт, когда рабочая область ещё не приехала, считать нужно
/// хоть по чему-то. Полоса меню — то единственное, что можно назвать числом:
/// Dock прячется, переезжает и меняет толщину.
const MENUBAR_PT: i32 = 25;

/// Наименьшая ширина окна: 80 колонок плюс рамка.
fn min_width() -> i32 {
    (f64::from(MIN_COLS) * COL_PT) as i32 + CHROME_PT
}

/// Наибольшая ширина окна: 120 колонок плюс рамка.
fn max_width() -> i32 {
    (f64::from(MAX_COLS) * COL_PT) as i32 + CHROME_PT
}

/// Запасная рабочая область: экран без полосы меню.
///
/// Годится ровно до того такта, когда приедет настоящая. Dock здесь не вычтен —
/// вычесть его числом нельзя.
pub fn work_area(screen: Bounds) -> Bounds {
    Bounds {
        x: screen.x,
        y: screen.y + MENUBAR_PT,
        width: screen.width,
        height: screen.height - MENUBAR_PT,
    }
}

/// Разложить `n` окон по рабочей области. Порядок ответа — порядок окон.
///
/// На входе именно рабочая область, а не экран: что из экрана вычесть, знает
/// платформа (полосу меню и Dock отдаёт `NSScreen::visibleFrame`), и знание это
/// здесь не повторяется — повторённое, оно разошлось бы с настоящим на первом
/// же переезде Dock.
pub fn arrange(mode: Layout, work: Bounds, n: usize) -> Vec<Bounds> {
    if n == 0 || work.width <= 0 || work.height <= 0 {
        return Vec::new();
    }
    match mode {
        Layout::Tile => tile(work, n),
        Layout::Cascade => cascade(work, n),
    }
}

/// Ступенька каскада — вправо и вниз разом.
///
/// Высоты заголовка хватило бы, чтобы окно опознать, но не чтобы за него
/// ухватиться: полсотни точек дают и заголовок целиком, и поле под курсор.
const STEP: i32 = 50;

/// Стопка со сдвигом вправо и вниз.
///
/// Окна одного размера: половина рабочей области по ширине, а по высоте —
/// сколько осталось после ступенек. Высота потому и считается от числа окон в
/// стопке: двум окнам ступенька нужна одна, и отдавать им столько же места,
/// сколько десяти, значит впустую резать высоту.
///
/// Ступенек помещается столько, чтобы окно не стало ниже половины рабочей
/// области и не уехало за правый край. Дальше стопка начинается заново от
/// левого верхнего угла: окон, которым не хватило ступенек, к этому времени
/// десяток, и экрану уже нечего им предложить, как ни считай.
fn cascade(work: Bounds, n: usize) -> Vec<Bounds> {
    let w = work.width / 2;
    let room_right = (work.width - w) / STEP;
    let room_down = (work.height / 2) / STEP;
    let per_stack = 1 + room_right.min(room_down).max(0);
    // Ступеньки считаются по стопке, а не по всему списку: в переполненной
    // стопке их ровно `per_stack - 1`, и высота у всех окон одна.
    let steps = (n as i32).min(per_stack) - 1;
    let h = work.height - steps * STEP;
    (0..n as i32)
        .map(|i| {
            let step = i % per_stack;
            Bounds {
                x: work.x + step * STEP,
                y: work.y + step * STEP,
                width: w,
                height: h,
            }
        })
        .collect()
}

/// Сетка без перекрытий, занимающая рабочую область целиком.
///
/// Колонки задаёт экран, а не число окон: три, если три дают терминал в
/// пределах 80–120 колонок; шире экран — колонок больше, уже — меньше. Окна
/// раскладываются по колонкам сверху вниз и делят высоту своей колонки поровну,
/// поэтому пустых клеток не остаётся: на двухколоночном экране три окна встают
/// как одно во всю высоту слева и два по половине справа.
///
/// Лишние окна достаются правым колонкам, а не левым. Первое окно списка —
/// самое верхнее в пикере, и ему достаётся колонка целиком: разложи мы остаток
/// слева, полную высоту получало бы последнее, до которого человеку дела
/// меньше всего.
fn tile(work: Bounds, n: usize) -> Vec<Bounds> {
    let cols = columns(work.width);
    // Ширина режется по 120 колонкам даже там, где экран позволяет больше:
    // растянутый на полтора метра терминал читать нечем — глаз не доносит
    // строку до конца. Колонок при этом ровно столько, чтобы до обрезки не
    // дошло, — она сторож, а не расчёт.
    let w = (work.width / cols).min(max_width());
    let base = n as i32 / cols;
    let rem = n as i32 % cols;
    let mut out = Vec::with_capacity(n);
    for col in 0..cols {
        // Окон меньше, чем колонок, — занимаются левые, по одному. Иначе
        // единственное окно уехало бы в правый угол.
        let in_col = if base == 0 {
            i32::from(col < rem)
        } else {
            base + i32::from(col >= cols - rem)
        };
        if in_col == 0 {
            continue;
        }
        let h = work.height / in_col;
        for row in 0..in_col {
            let y = work.y + row * h;
            // Нижнему окну колонки достаётся остаток от деления: без этого
            // между ним и краем оставалась бы щель в пару точек, и «занимает
            // весь экран» переставало быть правдой.
            let height = if row == in_col - 1 { work.y + work.height - y } else { h };
            out.push(Bounds { x: work.x + col * w, y, width: w, height });
        }
    }
    out
}

/// Сколько колонок в сетке. Зависит от экрана и только от него.
///
/// Три — идеал, и от него отступают только под нажимом экрана. Когда экран
/// узок настолько, что оба ограничения разом не выполнить (одна колонка уже
/// шире 120 знаков, а две — уже 80), побеждает нижнее: читать узкий терминал
/// хуже, чем широкий.
///
/// Числом окон сетка не сжимается: единственное окно на двухколоночном экране
/// занимает половину ширины, а не весь экран. Растянутое на весь, оно было бы
/// вдвое шире тех 120 знаков, ради которых колонки и считаются.
fn columns(width: i32) -> i32 {
    let most = (width / min_width()).max(1);
    let least = div_ceil(width, max_width()).max(1);
    3.max(least).min(most).max(1)
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

    /// Рабочая область ноутбука: 1440×900 без полосы меню.
    const LAPTOP: Bounds = Bounds { x: 0, y: 25, width: 1440, height: 875 };
    /// Рабочая область широкого монитора: 3440×1440 без полосы меню.
    const WIDE: Bounds = Bounds { x: 0, y: 25, width: 3440, height: 1415 };

    #[test]
    fn names_from_the_request_are_understood() {
        assert_eq!(Layout::from_name("tile"), Some(Layout::Tile));
        assert_eq!(Layout::from_name("  TILE  "), Some(Layout::Tile));
        assert_eq!(Layout::from_name("cascade"), Some(Layout::Cascade));
        assert_eq!(Layout::from_name("mosaic"), None);
    }

    #[test]
    fn nothing_to_place_is_not_a_layout() {
        assert!(arrange(Layout::Tile, WIDE, 0).is_empty());
    }

    #[test]
    fn the_spare_work_area_is_the_screen_without_the_menu_bar() {
        // Ею считают, пока не приехала настоящая — та, где вычтен и Dock.
        let screen = Bounds { x: 0, y: 0, width: 1440, height: 900 };
        assert_eq!(work_area(screen), LAPTOP);
    }

    #[test]
    fn the_layout_stays_inside_the_work_area() {
        // Рабочая область приходит снаружи и экраном не обязана быть: Dock
        // слева сдвигает её начало, снизу — укорачивает. Раскладка, считающая
        // от экрана, положила бы окна под Dock.
        let dock_on_the_left = Bounds { x: 80, y: 25, width: 1360, height: 875 };
        for mode in [Layout::Tile, Layout::Cascade] {
            for n in 1..=6 {
                for b in arrange(mode, dock_on_the_left, n) {
                    assert!(b.x >= dock_on_the_left.x, "{mode:?} {n}: {b:?}");
                    assert!(b.y >= dock_on_the_left.y, "{mode:?} {n}: {b:?}");
                    let right = dock_on_the_left.x + dock_on_the_left.width;
                    let bottom = dock_on_the_left.y + dock_on_the_left.height;
                    assert!(b.x + b.width <= right, "{mode:?} {n}: {b:?}");
                    assert!(b.y + b.height <= bottom, "{mode:?} {n}: {b:?}");
                }
            }
        }
    }

    // --- плитка ---

    #[test]
    fn three_windows_stand_in_one_row() {
        // Идеал из задачи: три вертикальных терминала на экран.
        let got = arrange(Layout::Tile, WIDE, 3);
        assert_eq!(got.len(), 3);
        let ys: Vec<i32> = got.iter().map(|b| b.y).collect();
        assert_eq!(ys, vec![WIDE.y; 3], "один ряд — одна высота");
        assert!(got[0].x < got[1].x && got[1].x < got[2].x, "порядок слева направо");
    }

    #[test]
    fn three_windows_on_two_columns_leave_no_empty_cell() {
        // Тот самый случай с ноутбука: первое окно занимает левую колонку
        // целиком, второе и третье делят правую. Сетка «два на два» оставила бы
        // внизу справа дыру в четверть экрана.
        let got = arrange(Layout::Tile, LAPTOP, 3);
        assert_eq!(got.len(), 3);
        assert_eq!(got[0].height, LAPTOP.height, "первому — колонка целиком");
        assert_eq!(got[0].width, LAPTOP.width / 2, "и ровно половина ширины");
        assert_eq!(got[1].x, got[2].x, "второе и третье — в одной колонке");
        assert!(got[1].x > got[0].x, "и колонка эта правая");
        assert_eq!(got[1].y, LAPTOP.y);
        assert_eq!(got[2].y, got[1].y + got[1].height, "делят её без щели");
    }

    #[test]
    fn the_tiles_cover_the_whole_work_area() {
        // «Заполнять весь экран» проверяется площадью: сумма площадей окон
        // равна площади рабочей области, а перекрытий нет (соседний тест).
        // Щель в пару точек от деления нацело сюда бы не прошла.
        for n in 2..=9 {
            for work in [LAPTOP, WIDE] {
                let got = arrange(Layout::Tile, work, n);
                if (n as i32) < columns(work.width) {
                    continue; // окон меньше, чем колонок, — пустые колонки законны
                }
                let covered: i64 = got.iter().map(|b| i64::from(b.width) * i64::from(b.height)).sum();
                let whole = i64::from(work.width) * i64::from(work.height);
                assert_eq!(covered, whole, "{n} окон на {}: {got:?}", work.width);
            }
        }
    }

    #[test]
    fn a_single_window_takes_one_column_on_the_left() {
        // Половина ширины, а не весь экран: растянутое на весь, окно было бы
        // вдвое шире тех 120 знаков, ради которых колонки и считаются.
        let got = arrange(Layout::Tile, LAPTOP, 1);
        assert_eq!(got[0], Bounds { x: LAPTOP.x, y: LAPTOP.y, width: 720, height: LAPTOP.height });
    }

    #[test]
    fn the_fullest_column_is_not_the_first_one() {
        // Первое окно списка — самое верхнее в пикере, и колонка ему достаётся
        // целиком. Раздай мы остаток слева, полную высоту получало бы
        // последнее, до которого человеку дела меньше всего.
        let got = arrange(Layout::Tile, LAPTOP, 5);
        let left: Vec<&Bounds> = got.iter().filter(|b| b.x == LAPTOP.x).collect();
        let right: Vec<&Bounds> = got.iter().filter(|b| b.x > LAPTOP.x).collect();
        assert_eq!((left.len(), right.len()), (2, 3), "{got:?}");
    }

    #[test]
    fn no_window_is_narrower_than_eighty_columns() {
        // Соль ограничения: терминал уже 80 знаков ломает вывод любой команды,
        // которая рисует таблицу.
        for n in 1..=8 {
            for work in [LAPTOP, WIDE] {
                for b in arrange(Layout::Tile, work, n) {
                    assert!(b.width >= min_width(), "{n} окон на {}: {b:?}", work.width);
                }
            }
        }
    }

    #[test]
    fn no_window_is_wider_than_a_hundred_and_twenty_columns() {
        for n in 1..=8 {
            for work in [LAPTOP, WIDE] {
                for b in arrange(Layout::Tile, work, n) {
                    assert!(b.width <= max_width(), "{n} окон на {}: {b:?}", work.width);
                }
            }
        }
    }

    #[test]
    fn a_narrow_screen_takes_fewer_columns_than_three() {
        // 1440 на три — по 480 точек, это 57 колонок. Три сюда не влезают, и
        // правило «не меньше 80» важнее идеала «три в ряд».
        assert_eq!(columns(LAPTOP.width), 2);
    }

    #[test]
    fn a_wide_screen_takes_more_columns_than_three() {
        // 3440 на три — по 1146 точек, это 140 колонок. Строку такой ширины
        // глаз не доносит до конца, поэтому колонок становится четыре.
        assert_eq!(columns(WIDE.width), 4);
    }

    #[test]
    fn tiles_never_overlap() {
        // Плитка тем и отличается от каскада: перекрытий нет — ни в колонке,
        // ни между колонками.
        for n in 1..=9 {
            for work in [LAPTOP, WIDE] {
                let got = arrange(Layout::Tile, work, n);
                for (i, a) in got.iter().enumerate() {
                    for b in got.iter().skip(i + 1) {
                        let apart = a.x + a.width <= b.x
                            || b.x + b.width <= a.x
                            || a.y + a.height <= b.y
                            || b.y + b.height <= a.y;
                        assert!(apart, "{n} окон: {a:?} лезет на {b:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn a_screen_too_narrow_for_the_rule_keeps_one_column() {
        // Экран, на котором одна колонка уже шире 120 знаков, а две — уже 80.
        // Оба ограничения разом не выполнить; побеждает нижнее, и окно
        // остаётся одно на ряд, шире обещанного.
        let narrow = Bounds { x: 0, y: 25, width: 1000, height: 800 };
        assert_eq!(columns(narrow.width), 1);
        let got = arrange(Layout::Tile, narrow, 2);
        assert!(got[0].y < got[1].y, "окна встали друг под другом в одной колонке");
        assert!(got[0].width >= min_width(), "ширина не опустилась ниже 80 колонок");
    }

    // --- каскад ---

    #[test]
    fn a_cascade_window_is_half_the_width() {
        let got = arrange(Layout::Cascade, WIDE, 3);
        assert!(got.iter().all(|b| b.width == WIDE.width / 2), "{got:?}");
    }

    #[test]
    fn a_cascade_takes_all_the_height_the_steps_leave() {
        // «Высота насколько возможно, за вычетом отступов»: три окна тратят две
        // ступеньки, и высота у них — рабочая область минус эти две.
        let got = arrange(Layout::Cascade, WIDE, 3);
        assert!(got.iter().all(|b| b.height == WIDE.height - 2 * STEP), "{got:?}");
    }

    #[test]
    fn two_windows_do_not_pay_for_steps_they_do_not_take() {
        // Высота считается по числу окон в стопке, а не по её вместимости:
        // двум окнам ступенька нужна одна.
        let two = arrange(Layout::Cascade, WIDE, 2);
        let five = arrange(Layout::Cascade, WIDE, 5);
        assert_eq!(two[0].height, WIDE.height - STEP);
        assert!(two[0].height > five[0].height, "у двоих окон высота больше, чем у пятерых");
    }

    #[test]
    fn every_cascade_window_can_be_grabbed_by_its_title() {
        // Соль каскада: заголовок — единственное, по чему человек выбирает окно
        // из стопки мышью, и полсотни точек дают не только увидеть его, но и
        // попасть в него курсором.
        let got = arrange(Layout::Cascade, WIDE, 5);
        for pair in got.windows(2) {
            assert_eq!(pair[1].y - pair[0].y, STEP, "{pair:?}");
            assert_eq!(pair[1].x - pair[0].x, STEP, "{pair:?}");
        }
    }

    #[test]
    fn the_cascade_starts_at_the_corner_of_the_work_area() {
        let got = arrange(Layout::Cascade, WIDE, 1);
        assert_eq!((got[0].x, got[0].y), (WIDE.x, WIDE.y));
        assert_eq!(got[0].height, WIDE.height, "одному окну ступеньки не нужны");
    }

    #[test]
    fn no_cascade_window_hangs_off_the_screen() {
        // Ступеньки уводят окна вправо и вниз, и без счёта места последнее
        // уехало бы за край — туда, где мышью его не достать.
        for n in 1..=40 {
            for work in [LAPTOP, WIDE] {
                for b in arrange(Layout::Cascade, work, n) {
                    assert!(b.x + b.width <= work.x + work.width, "{n}: {b:?}");
                    assert!(b.y + b.height <= work.y + work.height, "{n}: {b:?}");
                    assert!(b.height >= work.height / 2, "{n}: окно ниже половины: {b:?}");
                }
            }
        }
    }

    #[test]
    fn a_stack_that_ran_out_of_room_begins_anew() {
        // Ступенек тут помещается ровно восемь: 875/2 = 437 точек вниз.
        // Девятое окно начинает стопку заново от угла — предложить ему экрану
        // больше нечего.
        let got = arrange(Layout::Cascade, LAPTOP, 12);
        let wrap = (1..got.len()).find(|&i| got[i] == got[0]).expect("стопка началась заново");
        assert_eq!(wrap, 9, "ступенек помещается восемь: 875/2 = 437 точек вниз");
        assert!(got[wrap - 1].y > got[0].y, "до переноса стопка шла вниз");
    }

    #[test]
    fn a_second_screen_holds_the_cascade_too() {
        let right = Bounds { x: 1440, y: 25, width: 1440, height: 875 };
        let got = arrange(Layout::Cascade, right, 4);
        assert!(got.iter().all(|b| b.x >= right.x), "{got:?}");
    }
}
