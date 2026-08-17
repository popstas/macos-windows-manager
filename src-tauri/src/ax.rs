//! Окна терминалов глазами Accessibility.
//!
//! Всё, что трогает macOS, живёт здесь и только здесь: остальная логика — в
//! `mwm-core`, и её тесты гоняются на любой машине. Этот модуль тестами не
//! покрыт намеренно — проверять его нечем, кроме той самой машины, на которой
//! он и работает.

use mwm_core::geometry::{Bounds, Display};
use mwm_core::tracker::Seen;

#[cfg(target_os = "macos")]
mod imp {
    use super::{Bounds, Display, Seen};
    // Обёртка `accessibility` поверх `accessibility-sys` берёт на себя ровно
    // то, ради чего пришлось бы писать свой тип: `AXUIElement` там —
    // полноценный CF-тип, а значит `Clone` считает ссылки, а `PartialEq` —
    // это `CFEqual`, то самое «то же самое окно», на котором стоит реестр.
    use accessibility::{AXAttribute, AXUIElement};
    use accessibility_sys::{kAXTrustedCheckOptionPrompt, AXIsProcessTrustedWithOptions};
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::boolean::CFBoolean;
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::string::CFString;
    // Только геометрия: `CGPoint`/`CGSize` — сырые прямоугольники под AXValue,
    // `CGDisplay` — список экранов той же (Accessibility) системой координат,
    // без хождения через AppKit. Подробности — у `displays()` ниже.
    use core_graphics::display::CGDisplay;
    use core_graphics::geometry::{CGPoint, CGSize};
    use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication, NSWorkspace};

    /// Секунда на приложение — и это не перестраховка.
    ///
    /// Вызовы Accessibility синхронны и блокируются на неотвечающем
    /// приложении. Один подвисший терминал вешает весь такт, трекер молча
    /// перестаёт публиковать, а выглядит это как «менеджер умер».
    const MESSAGING_TIMEOUT_S: f32 = 1.0;

    pub fn trusted() -> bool {
        unsafe { AXIsProcessTrustedWithOptions(std::ptr::null()) }
    }

    pub fn prompt_for_trust() {
        let key = unsafe { CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt) };
        // core-foundation 0.10 не даёт `CFType: From<CFBoolean>` — оборачивать
        // приходится методом трейта `TCFType`, а не конструктором типа.
        let opts = CFDictionary::from_CFType_pairs(&[(key, CFBoolean::true_value().into_CFType())]);
        unsafe { AXIsProcessTrustedWithOptions(opts.as_concrete_TypeRef() as _) };
    }

    /// Кто есть кто между тактами.
    ///
    /// У окна macOS нет открытого стабильного номера: `AXUIElement` — ссылка, а
    /// не идентификатор, и порядок в `AXWindows` ничем не закреплён. Считать
    /// окно по порядковому номеру нельзя — перестановка выглядела бы сменой
    /// заголовка, и привязка сбрасывалась бы на ровном месте.
    ///
    /// Зато ссылки на одно и то же окно сравнимы: `CFEqual` отвечает «то же
    /// самое». Отсюда реестр: увидели впервые — выдали номер, увидели снова —
    /// нашли прежний. Частный `_AXUIElementGetWindow` дал бы номер сразу, но
    /// это непубличный интерфейс, а платить за него нечем: в публикуемом файле
    /// идентификатора окна нет вовсе, он нужен трекеру и никому больше.
    ///
    /// Рядом с номером хранится pid приложения-владельца. Спросить его у окна
    /// можно и потом, но здесь он уже известен даром — перечисление идёт по
    /// приложениям, — а подъёму он нужен обязательно: `AXRaise` поднимает окно
    /// внутри своего приложения, а вперёд приложение выводит уже AppKit.
    #[derive(Default)]
    pub struct Registry {
        known: Vec<(AXUIElement, u64)>,
        owners: std::collections::HashMap<u64, i32>,
        next: u64,
    }

    impl Registry {
        fn id_of(&mut self, el: &AXUIElement) -> u64 {
            if let Some((_, id)) = self.known.iter().find(|(k, _)| k == el) {
                return *id;
            }
            self.next += 1;
            self.known.push((el.clone(), self.next));
            self.next
        }

        /// Закрытые окна из реестра убираются: иначе он рос бы всю жизнь
        /// процесса, а сравнение с каждым его элементом — это и есть цена
        /// такта.
        fn retain_seen(&mut self, seen: &[AXUIElement]) {
            self.known.retain(|(k, _)| seen.iter().any(|s| s == k));
            let live: std::collections::HashSet<u64> =
                self.known.iter().map(|(_, id)| *id).collect();
            self.owners.retain(|id, _| live.contains(id));
        }
    }

    /// Заголовки окон всех запущенных терминалов из списка.
    pub fn list_windows(reg: &mut Registry, bundle_ids: &[String]) -> Vec<Seen> {
        let mut out = Vec::new();
        let mut alive: Vec<AXUIElement> = Vec::new();
        // Обёртки objc2-app-kit безопасны сами по себе: `unsafe` тут стоял зря и
        // стоил шести предупреждений на первой же сборке под macOS. Небезопасны
        // ниже вызовы Accessibility — там `unsafe` и остаётся.
        let ws = NSWorkspace::sharedWorkspace();
        let apps = ws.runningApplications();
        let front_pid = ws
            .frontmostApplication()
            .map(|a| a.processIdentifier())
            .unwrap_or(-1);
        for app in apps.iter() {
            let Some(id) = app.bundleIdentifier() else { continue };
            let id = id.to_string();
            if !bundle_ids.iter().any(|b| b.eq_ignore_ascii_case(&id)) {
                continue;
            }
            let pid = app.processIdentifier();
            let el = AXUIElement::application(pid);
            // Не поставился таймаут — сам такт всё равно продолжаем (лучше
            // рискнуть зависанием на одном приложении, чем вовсе пропустить
            // его), но узнать об этом стоит: до логгера в крейте руки не
            // дошли, а stderr виден и в `cargo run`, и в Console.app у
            // упакованного `.app`.
            if let Err(e) = el.set_messaging_timeout(MESSAGING_TIMEOUT_S) {
                eprintln!("mwm: set_messaging_timeout failed for pid {pid} ({id}): {e:?}");
            }
            // Фронтовое окно спрашивается только у фронтового приложения:
            // у остальных ответ есть, но означает он «последнее активное здесь»,
            // а нам нужно «то, на которое человек смотрит сейчас».
            let focused_window = if pid == front_pid {
                el.attribute(&AXAttribute::focused_window()).ok()
            } else {
                None
            };
            // Как зовут само приложение. Заголовок, равный этому имени, про
            // окно не сообщает ничего: так macOS отвечает, пока экран потушен —
            // у всех окон kitty заголовок разом становится «kitty». Что с
            // таким тактом делать, решает трекер (`Tracker::tick`), здесь
            // только факт.
            //
            // Имя берётся у приложения, а не сверяется со списком в конфиге:
            // в конфиге лежат идентификаторы пакетов (`net.kovidgoyal.kitty`),
            // а заголовком приезжает отображаемое имя, и выводить одно из
            // другого — гадание.
            let app_name = app.localizedName().map(|n| n.to_string()).unwrap_or_default();
            for w in windows_of(&el) {
                let Some(title) = title_of(&w) else { continue };
                if title.trim().is_empty() {
                    continue;
                }
                let id = reg.id_of(&w);
                reg.owners.insert(id, pid);
                // Сравнение по заголовку путало бы два окна с одинаковым
                // названием — обычное дело у терминалов на одном каталоге.
                // `focused_window` и `w` — те же `AXUIElement`, что и в
                // реестре, а их `PartialEq` — `CFEqual`: сравниваем сами
                // окна, а не то, как они называются.
                let focused = focused_window.as_ref() == Some(&w);
                // Геометрия спрашивается до перемещения `w` в `alive` — после
                // него ссылки уже нет.
                let bounds = bounds_of(&w);
                alive.push(w);
                // Сессия, названная в точности как терминал, признак получит
                // зря — но цена этому невелика: такт замирает, только когда
                // не назвалось ни одно окно, а рядом с такой сессией любое
                // другое окно снимет заморозку.
                let nameless = !app_name.is_empty() && title.trim() == app_name;
                // Имя приложения уезжает читателю: он различает по нему
                // терминалы в строке поимённо (kitty и iTerm2 иначе
                // неотличимы — пометка «окно есть» у них одна). Клонируется
                // на каждое окно приложения: строка одна на приложение, а
                // записей у него несколько.
                out.push(Seen { id, focused, title, bounds, nameless, app: app_name.clone() });
            }
        }
        reg.retain_seen(&alive);
        out
    }

    /// Окна приложения. Отказ — пустой список, а не ошибка: приложение могло
    /// закрыться между перечислением и вопросом, и это норма такта, а не сбой.
    fn windows_of(app: &AXUIElement) -> Vec<AXUIElement> {
        match app.attribute(&AXAttribute::windows()) {
            Ok(arr) => arr.iter().map(|w| w.clone()).collect(),
            Err(_) => Vec::new(),
        }
    }

    fn title_of(w: &AXUIElement) -> Option<String> {
        w.attribute(&AXAttribute::title()).ok().map(|t| t.to_string())
    }

    /// Где стоит окно. Отказ — `None`, а не ошибка: окно могло закрыться между
    /// перечислением и вопросом, и это норма такта.
    ///
    /// Позиция и размер спрашиваются порознь — они и есть два разных атрибута
    /// Accessibility. Не ответил ни один — координат нет вовсе: половина
    /// прямоугольника хуже, чем ничего, потому что по ней окно поставили бы
    /// не туда.
    ///
    /// Крейт `accessibility` 0.2.0 не знает ни атрибутов `AXPosition`/`AXSize`,
    /// ни самого типа `AXValue` вовсе — проверено в исходниках
    /// (`~/.cargo/registry/src/…/accessibility-0.2.0/src/attribute.rs` и
    /// `lib.rs`: там нет ни того, ни другого, `define_attributes!` их не
    /// перечисляет). Это не «разворачивается иначе», как в пробе 1 брифа, а
    /// «нет вовсе» — сразу проба 3: атрибут строится вручную через
    /// `AXAttribute::<CFType>::new` (`impl AXAttribute<CFType>` в том же
    /// `attribute.rs`), а получившийся `AXValue` разбирается руками через
    /// `accessibility_sys::AXValueGetType`/`AXValueGetValue` — другого пути в
    /// этой версии крейта нет.
    fn bounds_of(w: &AXUIElement) -> Option<Bounds> {
        let p = ax_value_attr(w, accessibility_sys::kAXPositionAttribute)?;
        let s = ax_value_attr(w, accessibility_sys::kAXSizeAttribute)?;
        let p: CGPoint = read_ax_value(&p, accessibility_sys::kAXValueTypeCGPoint)?;
        let s: CGSize = read_ax_value(&s, accessibility_sys::kAXValueTypeCGSize)?;
        Some(Bounds {
            x: p.x as i32,
            y: p.y as i32,
            width: s.width as i32,
            height: s.height as i32,
        })
    }

    /// Значение AXValue-атрибута как есть, нетипизированным `CFType`: у
    /// `AXPosition`/`AXSize` в крейте нет типизированной пары, а `CFType`
    /// принимает любой CF-объект — проверка типа внутри `attribute()`
    /// пропускается, когда `T::type_id() == CFType::type_id()`.
    fn ax_value_attr(w: &AXUIElement, name: &'static str) -> Option<CFType> {
        w.attribute(&AXAttribute::<CFType>::new(&CFString::from_static_string(name)))
            .ok()
    }

    /// Сырые байты `CGPoint`/`CGSize` из AXValue. `AXValueGetType` сверяет
    /// тип значения с ожидаемым — иначе `AXValueGetValue` прочитала бы чужую
    /// раскладку байт как свою.
    fn read_ax_value<T: Copy>(v: &CFType, kind: accessibility_sys::AXValueType) -> Option<T> {
        let value_ref = v.as_concrete_TypeRef() as accessibility_sys::AXValueRef;
        if unsafe { accessibility_sys::AXValueGetType(value_ref) } != kind {
            return None;
        }
        let mut out = std::mem::MaybeUninit::<T>::uninit();
        let ok = unsafe {
            accessibility_sys::AXValueGetValue(value_ref, kind, out.as_mut_ptr() as *mut std::ffi::c_void)
        };
        ok.then(|| unsafe { out.assume_init() })
    }

    /// Поднять окно и вывести вперёд его приложение.
    ///
    /// Два действия, а не одно. `AXRaise` поднимает окно внутри своего
    /// приложения — среди чужих окон оно так и останется позади. Вперёд
    /// приложение выводит AppKit, и без этого шага человек не увидел бы
    /// ничего, а трекер отчитался бы об успехе.
    ///
    /// Грамоты на передний план, вокруг которой построена вся Windows-ветка,
    /// здесь нет и не нужно: macOS решила этот вопрос разрешением
    /// Accessibility, выданным человеком один раз.
    pub fn raise(reg: &Registry, window_id: u64) -> Result<(), String> {
        let el = reg
            .known
            .iter()
            .find(|(_, id)| *id == window_id)
            .map(|(el, _)| el.clone())
            .ok_or("window is gone")?;
        let pid = *reg.owners.get(&window_id).ok_or("window owner is unknown")?;
        let action = CFString::from_static_string("AXRaise");
        let err = unsafe {
            accessibility_sys::AXUIElementPerformAction(
                el.as_concrete_TypeRef(),
                action.as_concrete_TypeRef(),
            )
        };
        if err != accessibility_sys::kAXErrorSuccess {
            return Err(format!("AXRaise failed: {err}"));
        }
        let app = NSRunningApplication::runningApplicationWithProcessIdentifier(pid);
        let app = app.ok_or("owner application is gone")?;
        // На macOS другие приложения могут быть впереди в момент просьбы —
        // активация обязана состояться, несмотря на это.
        // ActivateIgnoringOtherApps с macOS 14 не делает ничего — передаём пустой набор
        // параметров, смысл тот же. ActivateAllWindows тут не годится — он вынес бы
        // вперёд все окна терминала и обесценил бы AXRaise, который строкой выше
        // поднял ровно одно нужное.
        let activated = app.activateWithOptions(NSApplicationActivationOptions::empty());
        if !activated {
            return Err("failed to activate application".to_string());
        }
        Ok(())
    }

    /// Вывести вперёд само приложение.
    ///
    /// Нужно ровно по той же причине, что и вторая половина `raise()`, только
    /// для своего процесса: `show`, `unminimize` и `set_focus` двигают окно
    /// внутри приложения, а какое приложение впереди, решает AppKit. У
    /// `Accessory` это видно во всей красе — окно настроек открывается за
    /// терминалом, из которого человек только что полез в меню трея.
    ///
    /// Политику приложения этот вызов не трогает намеренно. Переключить её на
    /// `Regular` было бы вторым известным способом — и вернуло бы значок в док,
    /// ровно то, ради чего `Accessory` в `main()` и стоит.
    pub fn activate_self() -> Result<(), String> {
        let app = NSRunningApplication::currentApplication();
        // Пустой набор параметров — по той же причине, что в `raise()`:
        // `ActivateIgnoringOtherApps` с macOS 14 объявлен ничего не делающим.
        if !app.activateWithOptions(NSApplicationActivationOptions::empty()) {
            return Err("failed to activate this application".to_string());
        }
        Ok(())
    }

    /// Поставить окно в заданный прямоугольник.
    ///
    /// Позиция ставится раньше размера. Порядок не безразличен: некоторые
    /// приложения ужимают размер под текущий экран, и заданный до переезда он
    /// обрезался бы по старому месту.
    ///
    /// Проверка «атрибут вообще настраиваемый» стоит перед записью: у окна в
    /// полноэкранном режиме позиция только для чтения, и без проверки отказ
    /// выглядел бы отказом Accessibility вообще.
    ///
    /// Атрибуты и значения собираются так же, как читаются в `bounds_of`:
    /// типизированной пары для `AXPosition`/`AXSize` в крейте нет, поэтому
    /// `AXAttribute<CFType>` и ручная сборка AXValue. Метод проверки
    /// настраиваемости в этой версии крейта называется `is_settable`, а не
    /// `is_attribute_settable` — так он определён в
    /// `accessibility-0.2.0/src/ui_element.rs`, проба 2 из брифа (звать
    /// `accessibility_sys::AXUIElementIsAttributeSettable` напрямую) не
    /// понадобилась: типизированная обёртка нашлась под другим именем.
    pub fn place(reg: &Registry, window_id: u64, b: Bounds) -> Result<(), String> {
        let el = reg
            .known
            .iter()
            .find(|(_, id)| *id == window_id)
            .map(|(el, _)| el.clone())
            .ok_or("window is gone")?;
        let pos = AXAttribute::<CFType>::new(&CFString::from_static_string(
            accessibility_sys::kAXPositionAttribute,
        ));
        if !el.is_settable(&pos).unwrap_or(false) {
            return Err("window position is read-only (full screen?)".to_string());
        }
        let size = AXAttribute::<CFType>::new(&CFString::from_static_string(
            accessibility_sys::kAXSizeAttribute,
        ));
        let p = CGPoint::new(f64::from(b.x), f64::from(b.y));
        let s = CGSize::new(f64::from(b.width), f64::from(b.height));
        let pv = make_ax_value(accessibility_sys::kAXValueTypeCGPoint, &p)?;
        let sv = make_ax_value(accessibility_sys::kAXValueTypeCGSize, &s)?;
        el.set_attribute(&pos, pv).map_err(|e| format!("set position: {e:?}"))?;
        el.set_attribute(&size, sv).map_err(|e| format!("set size: {e:?}"))?;
        Ok(())
    }

    /// Собрать AXValue из сырых байт `CGPoint`/`CGSize`: крейт `accessibility`
    /// такой конструктор не даёт (см. `bounds_of` — типа `AXValue` там нет
    /// вовсе), `AXValueCreate` зовётся из `accessibility_sys` напрямую, как
    /// уже зовётся `AXUIElementPerformAction` в `raise`.
    fn make_ax_value<T>(kind: accessibility_sys::AXValueType, v: &T) -> Result<CFType, String> {
        let raw = unsafe {
            accessibility_sys::AXValueCreate(kind, (v as *const T).cast())
        };
        if raw.is_null() {
            return Err("AXValueCreate returned null".to_string());
        }
        Ok(unsafe { CFType::wrap_under_create_rule(raw as core_foundation::base::CFTypeRef) })
    }

    /// Экраны в той же системе координат, что и окна.
    ///
    /// `NSScreen.frame` считает от левого нижнего угла главного экрана, а
    /// Accessibility — от левого верхнего, и смешивать их нельзя. Бриф
    /// предлагал начать с `NSScreen::screens` и перейти на `CGDisplay`, если
    /// проба упрётся в главный поток. Здесь взят сразу `CGDisplay`, без
    /// пробы: такт трекера идёт в своём потоке
    /// (`std::thread::spawn(move || run_tracker(worker))` в `main.rs`), а не
    /// в главном, `NSScreen` — API главного потока, а `CGDisplay::bounds()`
    /// его не требует и, по документации в самих исходниках
    /// (`core-graphics-0.24.0/src/display.rs`: «Returns the bounds of a
    /// display in the global display coordinate space»), уже отдаёт
    /// координаты в той же системе, что и Accessibility — переворачивать `y`
    /// самим не нужно.
    pub fn displays() -> Vec<Display> {
        let ids = match CGDisplay::active_displays() {
            Ok(ids) => ids,
            Err(e) => {
                eprintln!("mwm: CGGetActiveDisplayList failed: {e}");
                return Vec::new();
            }
        };
        ids.into_iter()
            .map(|id| {
                let r = CGDisplay::new(id).bounds();
                Display {
                    bounds: Bounds {
                        x: r.origin.x as i32,
                        y: r.origin.y as i32,
                        width: r.size.width as i32,
                        height: r.size.height as i32,
                    },
                }
            })
            .collect()
    }
}

/// На не-macOS модуль отвечает пустотой: крейт должен собираться где угодно,
/// чтобы `cargo check` был доступен и не на маке.
#[cfg(not(target_os = "macos"))]
mod imp {
    use super::{Bounds, Display, Seen};
    #[derive(Default)]
    pub struct Registry;
    pub fn trusted() -> bool { false }
    pub fn prompt_for_trust() {}
    pub fn list_windows(_reg: &mut Registry, _bundle_ids: &[String]) -> Vec<Seen> { Vec::new() }
    pub fn raise(_reg: &Registry, _window_id: u64) -> Result<(), String> {
        Err("raise is available on macOS only".to_string())
    }
    pub fn activate_self() -> Result<(), String> {
        Err("activating the application is available on macOS only".to_string())
    }
    pub fn place(_reg: &Registry, _window_id: u64, _b: Bounds) -> Result<(), String> {
        Err("placing windows is available on macOS only".to_string())
    }
    pub fn displays() -> Vec<Display> { Vec::new() }
}

pub use imp::{
    activate_self, displays, list_windows, place, prompt_for_trust, raise, trusted, Registry,
};
