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
    use objc2_app_kit::NSWorkspace;

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
    #[derive(Default)]
    pub struct Registry {
        known: Vec<(AXUIElement, u64)>,
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
        }
    }

    /// Заголовки окон всех запущенных терминалов из списка.
    pub fn list_windows(reg: &mut Registry, bundle_ids: &[String]) -> Vec<Seen> {
        let mut out = Vec::new();
        let mut alive: Vec<AXUIElement> = Vec::new();
        let ws = unsafe { NSWorkspace::sharedWorkspace() };
        let apps = unsafe { ws.runningApplications() };
        let front_pid = unsafe { ws.frontmostApplication() }
            .map(|a| unsafe { a.processIdentifier() })
            .unwrap_or(-1);
        for app in apps.iter() {
            let Some(id) = (unsafe { app.bundleIdentifier() }) else { continue };
            let id = id.to_string();
            if !bundle_ids.iter().any(|b| b.eq_ignore_ascii_case(&id)) {
                continue;
            }
            let pid = unsafe { app.processIdentifier() };
            let el = AXUIElement::application(pid);
            let _ = el.set_messaging_timeout(MESSAGING_TIMEOUT_S);
            // Фронтовое окно спрашивается только у фронтового приложения:
            // у остальных ответ есть, но означает он «последнее активное здесь»,
            // а нам нужно «то, на которое человек смотрит сейчас».
            let focused_title = if pid == front_pid {
                el.attribute(&AXAttribute::focused_window())
                    .ok()
                    .and_then(|w| title_of(&w))
            } else {
                None
            };
            for w in windows_of(&el) {
                let Some(title) = title_of(&w) else { continue };
                if title.trim().is_empty() {
                    continue;
                }
                let id = reg.id_of(&w);
                alive.push(w);
                out.push(Seen {
                    id,
                    focused: focused_title.as_deref() == Some(title.as_str()),
                    title,
                });
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
}

pub use imp::{list_windows, prompt_for_trust, trusted, Registry};
