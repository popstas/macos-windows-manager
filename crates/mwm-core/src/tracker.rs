//! Тик трекера: какое окно какой сессии принадлежит.

use crate::title::strip_decoration;
use std::collections::{BTreeMap, HashMap};

/// Окно, каким его увидел платформенный слой на этом такте.
#[derive(Debug, Clone)]
pub struct Seen {
    /// Устойчив в пределах жизни трекера и больше нигде не нужен: в
    /// публикуемом файле идентификатора окна нет вовсе.
    pub id: u64,
    pub title: String,
    pub focused: bool,
}

/// Что уезжает читателю про одну сессию.
#[derive(Debug, Clone, PartialEq)]
pub struct Bound {
    pub session_id: String,
    pub title: String,
    pub last_seen_ms: u64,
    pub focused_at_ms: u64,
}

#[derive(Debug, Default, Clone)]
struct Tracked {
    title: String,
    ticks: u32,
    stable: Option<String>,
    session_id: Option<String>,
}

/// Слот переживает закрытие окна: он затем и заведён, чтобы вернуть сессию на
/// прежнее место и удержать привязку, пока заголовок меняется.
#[derive(Debug, Default, Clone)]
struct Slot {
    focused_at_ms: u64,
}

pub struct Tracker {
    stable_ticks: u32,
    windows: HashMap<u64, Tracked>,
    slots: HashMap<String, Slot>,
    bound: BTreeMap<String, Bound>,
    unresolved: Vec<String>,
}

impl Tracker {
    pub fn new(stable_ticks: u32) -> Self {
        Self {
            stable_ticks: stable_ticks.max(1),
            windows: HashMap::new(),
            slots: HashMap::new(),
            bound: BTreeMap::new(),
            unresolved: Vec::new(),
        }
    }

    /// Один такт: что видно на экране, что об этом знает дамп, который час.
    pub fn tick(&mut self, seen: &[Seen], index: &BTreeMap<String, String>, now_ms: u64) {
        // Двойники по заголовку: побеждает больший идентификатор — окно новее.
        // Остальные остаются непривязанными, чтобы не драться за один слот.
        let mut winners: HashMap<&str, u64> = HashMap::new();
        for w in seen {
            let e = winners.entry(w.title.as_str()).or_insert(w.id);
            if w.id > *e {
                *e = w.id;
            }
        }

        let live: Vec<u64> = seen.iter().map(|w| w.id).collect();
        self.windows.retain(|id, _| live.contains(id));
        self.bound.clear();
        self.unresolved.clear();

        for w in seen {
            let t = self.windows.entry(w.id).or_default();
            if t.title == w.title {
                t.ticks += 1;
            } else {
                t.title = w.title.clone();
                t.ticks = 1;
            }
            if t.ticks >= self.stable_ticks {
                t.stable = Some(w.title.clone());
            }
            let Some(stable) = t.stable.clone() else { continue };
            if winners.get(w.title.as_str()) != Some(&w.id) {
                continue;
            }
            let key = strip_decoration(&stable);
            if let Some(sid) = index.get(&key) {
                t.session_id = Some(sid.clone());
            } else if !key.is_empty() {
                // Заголовок устоялся, а сессии под него нет — значит, дамп
                // пора освежить. Ходить за ним на каждом такте незачем: он
                // меняется на каждый ответ агента.
                //
                // Спрашивают и про окно, у которого привязка уже есть, и это
                // не расточительство. В том же терминале запускают claude
                // заново: id новый, окно прежнее. Привяжись мы однажды и
                // перестань спрашивать — строка навсегда осталась бы на
                // прошлой сессии, а работающая не была бы видна вовсе.
                self.unresolved.push(key.clone());
            }
            let Some(sid) = t.session_id.clone() else { continue };
            let slot = self.slots.entry(sid.clone()).or_default();
            if w.focused {
                slot.focused_at_ms = now_ms;
            }
            self.bound.insert(
                sid.clone(),
                Bound {
                    session_id: sid,
                    title: key,
                    last_seen_ms: now_ms,
                    focused_at_ms: slot.focused_at_ms,
                },
            );
        }
    }

    /// Окна текущего такта, привязанные к сессиям.
    pub fn bound(&self) -> BTreeMap<String, Bound> {
        self.bound.clone()
    }

    /// Устоявшиеся заголовки, которым дамп не нашёл сессии.
    pub fn unresolved(&self) -> Vec<String> {
        self.unresolved.clone()
    }

    /// Вернуть сессию в непрочитанное: обнулить отметку взгляда.
    ///
    /// Правится и слот, и текущая привязка. Слот — потому что он источник
    /// правды: `tick` собирает `bound` заново из слотов, и без обнуления слота
    /// отмотка прожила бы ровно один такт. `bound` — потому что порядок
    /// вызовов не наш: просьба приходит из другого потока в любой момент, и
    /// файл, записанный между отмоткой и следующим тиком, обязан её показывать.
    ///
    /// Отпечаток расклада считает `focused_at_ms`, поэтому просить о записи
    /// отдельно не нужно: `should_write` заметит изменение сам.
    ///
    /// Незнакомая сессия — молчание, а не ошибка: просьба едет с чужой машины,
    /// и окно могли закрыть, пока она ехала.
    pub fn mark_unread(&mut self, session_id: &str) {
        if let Some(slot) = self.slots.get_mut(session_id) {
            slot.focused_at_ms = 0;
        }
        if let Some(b) = self.bound.get_mut(session_id) {
            b.focused_at_ms = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    const SID: &str = "aaaaaaaa-1111-2222-3333-444444444444";

    fn index(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(t, s)| (t.to_string(), s.to_string())).collect()
    }

    fn seen(id: u64, title: &str) -> Seen {
        Seen { id, title: title.to_string(), focused: false }
    }

    #[test]
    fn binding_waits_for_the_title_to_settle() {
        // Вход в сессию перещёлкивает заголовок два-три раза подряд — шелл,
        // claude, имя сессии. Привязка по первому же значению села бы на
        // промежуточное, и окно осталось бы за чужой сессией до перезапуска.
        let mut t = Tracker::new(2);
        let idx = index(&[("ccfzf", SID)]);
        t.tick(&[seen(1, "ccfzf")], &idx, 1_000);
        assert!(t.bound().is_empty(), "одного такта мало");
        t.tick(&[seen(1, "ccfzf")], &idx, 2_000);
        assert_eq!(t.bound().keys().collect::<Vec<_>>(), vec![SID]);
    }

    #[test]
    fn binding_survives_the_title_changing() {
        // Сессия правит заголовок терминала на каждый ответ, а дамп отстаёт до
        // тридцати секунд. Слот — это ровно то, что удерживает окно за сессией
        // в промежутке; без него окно мигало бы в списке, пока сессия работает.
        let mut t = Tracker::new(2);
        let idx = index(&[("ccfzf", SID)]);
        t.tick(&[seen(1, "ccfzf")], &idx, 1_000);
        t.tick(&[seen(1, "ccfzf")], &idx, 2_000);
        t.tick(&[seen(1, "writing the plan")], &BTreeMap::new(), 3_000);
        t.tick(&[seen(1, "writing the plan")], &BTreeMap::new(), 4_000);
        assert_eq!(t.bound().keys().collect::<Vec<_>>(), vec![SID], "окно осталось за сессией");
    }

    #[test]
    fn closed_window_leaves_the_published_list() {
        // Публикуются окна текущего такта, а не слоты: слот переживает закрытие
        // окна намеренно, и по слотам файл рассказывал бы про окна, которых на
        // экране нет.
        let mut t = Tracker::new(1);
        let idx = index(&[("ccfzf", SID)]);
        t.tick(&[seen(1, "ccfzf")], &idx, 1_000);
        assert_eq!(t.bound().len(), 1);
        t.tick(&[], &idx, 2_000);
        assert!(t.bound().is_empty(), "окна нет — и записи нет");
    }

    #[test]
    fn unknown_settled_title_is_reported() {
        // Ходить за дампом на каждом такте незачем: заголовок меняется на
        // каждый ответ агента. Спрашивают его тогда, когда есть о чём — вот об
        // этом списке.
        let mut t = Tracker::new(1);
        t.tick(&[seen(1, "brand new session")], &BTreeMap::new(), 1_000);
        assert_eq!(t.unresolved(), vec!["brand new session".to_string()]);
    }

    #[test]
    fn a_restarted_session_takes_its_window_back() {
        // В том же терминале запустили claude заново: id новый, окно прежнее.
        // Привязавшись однажды и перестав спрашивать дамп, трекер держал бы
        // строку на прошлой сессии вечно, а работающую не показывал бы вовсе.
        let mut t = Tracker::new(1);
        t.tick(&[seen(1, "ccfzf")], &index(&[("ccfzf", SID)]), 1_000);
        assert_eq!(t.bound().keys().collect::<Vec<_>>(), vec![SID]);
        // Заголовок сменился, дамп ещё не знает нового имени — про него и
        // спрашивают.
        t.tick(&[seen(1, "ccfzf-2")], &index(&[("ccfzf", SID)]), 2_000);
        assert_eq!(t.unresolved(), vec!["ccfzf-2".to_string()]);
        // Дамп освежился — окно переехало к новой сессии.
        const SID2: &str = "cccccccc-1111-2222-3333-444444444444";
        t.tick(&[seen(1, "ccfzf-2")], &index(&[("ccfzf-2", SID2)]), 3_000);
        assert_eq!(t.bound().keys().collect::<Vec<_>>(), vec![SID2]);
    }

    #[test]
    fn focus_stamp_is_set_when_the_window_becomes_frontmost() {
        // «Просмотрено» приезжает отсюда и ниоткуда больше: переход взгляда на
        // окно виден только трекеру.
        let mut t = Tracker::new(1);
        let idx = index(&[("ccfzf", SID)]);
        t.tick(&[seen(1, "ccfzf")], &idx, 1_000);
        assert_eq!(t.bound()[SID].focused_at_ms, 0);
        t.tick(&[Seen { id: 1, title: "ccfzf".into(), focused: true }], &idx, 5_000);
        assert_eq!(t.bound()[SID].focused_at_ms, 5_000);
        t.tick(&[seen(1, "ccfzf")], &idx, 9_000);
        assert_eq!(t.bound()[SID].focused_at_ms, 5_000, "отметка не откатывается");
    }

    #[test]
    fn twins_by_title_go_to_the_newer_window() {
        // Два окна с одним заголовком законны. Драться за один слот им нельзя:
        // побеждает окно с большим идентификатором — оно новее.
        let mut t = Tracker::new(1);
        // Даём двойникам заголовок, которого нет в индексе: только побеждающее
        // окно (с большим id) войдёт в unresolved(), проигравшее — нет.
        t.tick(&[seen(1, "unknown"), seen(9, "unknown")], &BTreeMap::new(), 1_000);
        let unresolved = t.unresolved();
        // Если убрать guard по winners, оба окна войдут в unresolved, и это
        // утверждение упадёт.
        assert_eq!(unresolved.len(), 1, "только побеждающее окно в unresolved: {unresolved:?}");
        assert_eq!(unresolved[0], "unknown");
    }

    #[test]
    fn only_settled_titles_enter_unresolved() {
        // unresolved() должна возвращать только истинно устоявшиеся заголовки,
        // а не текущие. На первом такте с стабилизацией stable_ticks=2
        // заголовок, которого нет в индексе, не должен появиться в unresolved().
        // Это предотвращает шум на каждом мелькании заголовка при входе в
        // сессию.
        let mut t = Tracker::new(2);
        // Первый такт: заголовок не в индексе, но не устоялся.
        t.tick(&[seen(1, "not-in-index")], &BTreeMap::new(), 1_000);
        // Если убрать guard `let Some(stable) = t.stable.clone() else { continue }`,
        // это утверждение упадёт, и мы будем спрашивать дамп на каждый вход.
        assert!(t.unresolved().is_empty(), "первый такт: заголовок не устоялся");
        // Второй такт: заголовок повторился, теперь он устоялся.
        t.tick(&[seen(1, "not-in-index")], &BTreeMap::new(), 2_000);
        assert_eq!(t.unresolved(), vec!["not-in-index".to_string()], "второй такт: заголовок устоялся");
    }

    #[test]
    fn decorated_title_matches_the_bare_one() {
        // Значок состояния перед заголовком снимается перед сравнением.
        let mut t = Tracker::new(1);
        let idx = index(&[("ccfzf", SID)]);
        t.tick(&[seen(1, "✳ ccfzf")], &idx, 1_000);
        let bound = t.bound();
        assert_eq!(bound.keys().collect::<Vec<_>>(), vec![SID]);
        // Проверяем, что title в Bound содержит очищенный заголовок, а не украшенный.
        // Если убрать strip_decoration(), это утверждение упадёт.
        assert_eq!(bound[SID].title, "ccfzf", "Bound.title должна содержать очищенный заголовок");
    }

    #[test]
    fn mark_unread_rewinds_the_focus_stamp() {
        // Своя отметка в seen.json у пикера бессильна: трекерная почти всегда
        // свежее и побеждает по максимуму. Отматывать надо ту, что перебивает.
        let mut t = Tracker::new(1);
        let idx = index(&[("ccfzf", SID)]);
        t.tick(&[Seen { id: 1, title: "ccfzf".into(), focused: true }], &idx, 5_000);
        assert_eq!(t.bound()[SID].focused_at_ms, 5_000);
        t.mark_unread(SID);
        assert_eq!(t.bound()[SID].focused_at_ms, 0, "отметка отмотана сразу, а не к следующему такту");
    }

    #[test]
    fn a_rewound_stamp_stays_rewound_while_the_window_is_not_watched() {
        // Слот переживает такт; не обнули мы его, следующий же тик вернул бы
        // прежнее значение, и отмотка выглядела бы сработавшей ровно на секунду.
        let mut t = Tracker::new(1);
        let idx = index(&[("ccfzf", SID)]);
        t.tick(&[Seen { id: 1, title: "ccfzf".into(), focused: true }], &idx, 5_000);
        t.mark_unread(SID);
        t.tick(&[seen(1, "ccfzf")], &idx, 6_000);
        assert_eq!(t.bound()[SID].focused_at_ms, 0);
    }

    #[test]
    fn looking_at_the_window_again_marks_it_seen_again() {
        // «Просмотрено» и значит «взгляд на нём сейчас». Возврат взгляда обязан
        // ставить отметку заново — иначе отмотка была бы не отметкой, а
        // запретом.
        let mut t = Tracker::new(1);
        let idx = index(&[("ccfzf", SID)]);
        t.tick(&[Seen { id: 1, title: "ccfzf".into(), focused: true }], &idx, 5_000);
        t.mark_unread(SID);
        t.tick(&[Seen { id: 1, title: "ccfzf".into(), focused: true }], &idx, 7_000);
        assert_eq!(t.bound()[SID].focused_at_ms, 7_000);
    }

    #[test]
    fn mark_unread_of_an_unknown_session_is_quiet() {
        // Просьба приезжает с чужой машины и может опоздать: сессию закрыли
        // между опросом и нажатием. Это норма, а не сбой.
        let mut t = Tracker::new(1);
        t.mark_unread("нет-такой");
        assert!(t.bound().is_empty());
    }
}
