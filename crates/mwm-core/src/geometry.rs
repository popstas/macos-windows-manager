//! Прямоугольники окон и экранов.
//!
//! Система координат — глобальная, с началом в левом верхнем углу главного
//! экрана: такую отдаёт Accessibility. `NSScreen` считает от левого нижнего, и
//! смешивать их нельзя — список экранов поэтому берётся тем же способом, каким
//! читаются окна, а не через AppKit.

/// Прямоугольник в глобальных координатах.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Экран — тот же прямоугольник, отдельным типом ради читаемости сигнатур.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Display {
    pub bounds: Bounds,
}

/// Какую долю площади окна должны накрывать экраны, чтобы положение считалось
/// годным. Половина, а не «видно хоть сколько-нибудь»: окно, торчащее с экрана
/// на три четверти, человеку так же бесполезно, как уехавшее целиком.
const VISIBLE_NUM: i64 = 1;
const VISIBLE_DEN: i64 = 2;

/// Подогнать положение окна под текущие экраны.
pub fn clamp_to_displays(b: Bounds, displays: &[Display]) -> Bounds {
    if displays.is_empty() {
        return b;
    }
    let area = i64::from(b.width.max(0)) * i64::from(b.height.max(0));
    if area == 0 {
        return b;
    }
    let covered: i64 = displays.iter().map(|d| overlap(&b, &d.bounds)).sum();
    if covered * VISIBLE_DEN >= area * VISIBLE_NUM {
        return b;
    }
    // Возвращаем на экран с наибольшим перекрытием, а не на первый попавшийся:
    // иначе окно, съехавшее на стык двух экранов, прыгало бы через весь стол.
    let best = displays
        .iter()
        .max_by_key(|d| overlap(&b, &d.bounds))
        .map(|d| d.bounds)
        .unwrap_or(b);
    let width = b.width.min(best.width);
    let height = b.height.min(best.height);
    Bounds {
        x: b.x.max(best.x).min(best.x + best.width - width),
        y: b.y.max(best.y).min(best.y + best.height - height),
        width,
        height,
    }
}

/// Площадь пересечения. В `i64`, потому что произведение двух `i32` из него
/// выходит: экран 8K на паре мониторов даёт число за пределами `i32`.
fn overlap(a: &Bounds, b: &Bounds) -> i64 {
    let x = (a.x + a.width).min(b.x + b.width) - a.x.max(b.x);
    let y = (a.y + a.height).min(b.y + b.height) - a.y.max(b.y);
    if x <= 0 || y <= 0 {
        return 0;
    }
    i64::from(x) * i64::from(y)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(x: i32, y: i32, width: i32, height: i32) -> Bounds {
        Bounds { x, y, width, height }
    }

    fn screens(list: &[Bounds]) -> Vec<Display> {
        list.iter().map(|&bounds| Display { bounds }).collect()
    }

    #[test]
    fn a_window_on_screen_is_left_alone() {
        let d = screens(&[b(0, 0, 1920, 1080)]);
        let w = b(100, 100, 800, 600);
        assert_eq!(clamp_to_displays(w, &d), w);
    }

    #[test]
    fn a_window_mostly_on_screen_is_left_alone() {
        // Окно, свисающее с края на четверть, человек видит и может подвинуть
        // сам. Дёргать его — значит спорить с тем, как он его поставил.
        let d = screens(&[b(0, 0, 1000, 1000)]);
        let w = b(750, 0, 400, 400);
        assert_eq!(clamp_to_displays(w, &d), w, "накрыто 250 из 400 по ширине — больше половины площади");
    }

    #[test]
    fn a_window_off_screen_comes_back() {
        // Внешнего экрана не стало, окно осталось на его координатах. Мышкой
        // такое не вернуть — его вообще не видно.
        let d = screens(&[b(0, 0, 1000, 1000)]);
        let w = b(3000, 200, 400, 300);
        let got = clamp_to_displays(w, &d);
        assert_eq!(got, b(600, 200, 400, 300), "прижато к правому краю, размер сохранён");
    }

    #[test]
    fn a_window_larger_than_the_screen_shrinks_to_fit() {
        let d = screens(&[b(0, 0, 800, 600)]);
        let w = b(5000, 5000, 1600, 1200);
        assert_eq!(clamp_to_displays(w, &d), b(0, 0, 800, 600));
    }

    #[test]
    fn the_screen_with_the_most_overlap_wins() {
        // Два экрана с зазором между ними, окно уехало в зазор и цепляет оба
        // краями. Накрыто меньше половины площади — значит возвращать; вернуть
        // надо на тот экран, где окна больше, иначе оно прыгает через весь стол.
        //
        // Зазор между экранами обязателен: на смежных экранах окно, уехавшее на
        // стык, накрыто целиком, ветка выбора экрана не исполняется вовсе, и
        // тест зеленел бы, ничего не проверив.
        //
        // Случая два, и они зеркальны намеренно: с одним тест не отличил бы
        // правило «побеждает наибольшее перекрытие» от правила «побеждает
        // первый экран в списке».
        let d = screens(&[b(0, 0, 1000, 1000), b(1500, 0, 1000, 1000)]);

        // Слева 300 столбцов из 900, справа 100 — накрыто 160 000 из 360 000.
        assert_eq!(
            clamp_to_displays(b(700, 100, 900, 400), &d),
            b(100, 100, 900, 400),
            "перекрытие больше слева — окно вернулось на левый экран",
        );

        // Зеркально: слева 100 столбцов, справа 300.
        assert_eq!(
            clamp_to_displays(b(900, 100, 900, 400), &d),
            b(1500, 100, 900, 400),
            "перекрытие больше справа — правило «первый в списке» выбрало бы левый",
        );
    }

    #[test]
    fn no_screens_means_no_opinion() {
        // Экранов не видно вовсе — клампить не к чему. Отказ от расстановки был
        // бы хуже: окно осталось бы там, куда его положила система.
        let w = b(100, 100, 800, 600);
        assert_eq!(clamp_to_displays(w, &[]), w);
    }

    #[test]
    fn a_zero_sized_window_is_left_alone() {
        // Размер приезжает от Accessibility и может оказаться нулевым, если
        // окно как раз закрывается. Делить на ноль в подсчёте доли нельзя, а
        // трогать такое окно незачем.
        let d = screens(&[b(0, 0, 1000, 1000)]);
        let w = b(5000, 5000, 0, 0);
        assert_eq!(clamp_to_displays(w, &d), w);
    }
}
