//! Окна терминалов глазами Accessibility.
//!
//! Всё, что трогает macOS, живёт здесь и только здесь: остальная логика — в
//! `mwm-core`, и её тесты гоняются на любой машине. Этот модуль тестами не
//! покрыт намеренно — проверять его нечем, кроме той самой машины, на которой
//! он и работает.

use mwm_core::tracker::Seen;

#[cfg(target_os = "macos")]
mod imp {
    use super::Seen;
    // Обёртка `accessibility` поверх `accessibility-sys` берёт на себя ровно
    // то, ради чего пришлось бы писать свой тип: `AXUIElement` там —
    // полноценный CF-тип, а значит `Clone` считает ссылки, а `PartialEq` —
    // это `CFEqual`, то самое «то же самое окно», на котором стоит реестр.
    use accessibility::{AXAttribute, AXUIElement};
    use accessibility_sys::{kAXTrustedCheckOptionPrompt, AXIsProcessTrustedWithOptions};
    use core_foundation::base::TCFType;
    use core_foundation::boolean::CFBoolean;
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::string::CFString;
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
                alive.push(w);
                out.push(Seen { id, focused, title });
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
}

/// На не-macOS модуль отвечает пустотой: крейт должен собираться где угодно,
/// чтобы `cargo check` был доступен и не на маке.
#[cfg(not(target_os = "macos"))]
mod imp {
    use super::Seen;
    #[derive(Default)]
    pub struct Registry;
    pub fn trusted() -> bool { false }
    pub fn prompt_for_trust() {}
    pub fn list_windows(_reg: &mut Registry, _bundle_ids: &[String]) -> Vec<Seen> { Vec::new() }
    pub fn raise(_reg: &Registry, _window_id: u64) -> Result<(), String> {
        Err("raise is available on macOS only".to_string())
    }
}

pub use imp::{list_windows, prompt_for_trust, raise, trusted, Registry};
