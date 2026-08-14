//! Тик трекера: какое окно какой сессии принадлежит.

use crate::geometry::Bounds;
use crate::index::SessionRef;
use crate::state::SlotState;
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
    /// Где окно стоит сейчас. `None` — платформа не ответила; это норма такта,
    /// а не сбой, и стоит она ровно того, что положение в этот такт не
    /// обновится.
    pub bounds: Option<Bounds>,
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
    /// Устойчивость положения меряется здесь, а не в слоте: слот заводится
    /// лишь когда заголовок уже устоялся и сессия найдена, а окно видно (и
    /// может уже стоять на месте) раньше. Считай так с самого первого такта —
    /// и к моменту, когда слот наконец заведётся, устойчивость может
    /// оказаться уже накоплена.
    pending: Option<Bounds>,
    pending_ticks: u32,
    /// Расстановка предлагается окну ровно один раз за его жизнь — на том
    /// такте, когда для него наконец нашёлся слот, а не на том, когда оно
    /// впервые попало в список: заголовок и сессия могут устояться на такты
    /// позже появления самого окна. Ставится при заведении записи в `tick` —
    /// `true`, если трекер уже работал (иначе окно было открыто до него и
    /// расстановки не заслужило), `false` для перезапуска намеренно.
    awaiting_placement: bool,
}

/// Слот переживает закрытие окна: он затем и заведён, чтобы вернуть сессию на
/// прежнее место и удержать привязку, пока заголовок меняется.
///
/// Устойчивое положение и то, что видно сейчас, разведены намеренно. Пока окно
/// тащат мышкой, координаты меняются каждый такт, а запоминать их значило бы
/// звать `fsync` на каждый такт перетаскивания.
#[derive(Debug, Default, Clone)]
struct Slot {
    focused_at_ms: u64,
    bounds: Option<Bounds>,
    title: String,
    cwd: String,
    last_seen_ms: u64,
}

pub struct Tracker {
    stable_ticks: u32,
    windows: HashMap<u64, Tracked>,
    slots: HashMap<String, Slot>,
    bound: BTreeMap<String, Bound>,
    unresolved: Vec<String>,
    /// Первый такт после запуска не расставляет ничего. Отдельное правило, а не
    /// следствие: на первом такте прошлого такта нет, и все открытые окна
    /// выглядят только что появившимися.
    started: bool,
    placements: Vec<(u64, Bounds)>,
    dirty: bool,
}

impl Tracker {
    pub fn new(stable_ticks: u32) -> Self {
        Self {
            stable_ticks: stable_ticks.max(1),
            windows: HashMap::new(),
            slots: HashMap::new(),
            bound: BTreeMap::new(),
            unresolved: Vec::new(),
            started: false,
            placements: Vec::new(),
            dirty: false,
        }
    }

    /// Один такт: что видно на экране, что об этом знает дамп, который час.
    pub fn tick(&mut self, seen: &[Seen], index: &BTreeMap<String, SessionRef>, now_ms: u64) {
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
        self.placements.clear();
        // `started` смотрим до того, как выставим его в true чуть ниже: этим
        // же значением заводимые в этом такте окна решают, заслужили ли они
        // предложение расстановки.
        let started_before = self.started;
        self.started = true;

        for w in seen {
            let is_new_window = !self.windows.contains_key(&w.id);
            let t = self.windows.entry(w.id).or_default();
            if is_new_window {
                // Окно появилось только что. Если трекер уже работал —
                // заслужило одно предложение расстановки, когда бы для него
                // ни нашёлся слот. Если это первый такт после запуска —
                // предложения не будет никогда: все окна, видимые тогда, были
                // открыты не трекером, и правило 2 требует их не трогать.
                t.awaiting_placement = started_before;
            }
            if t.title == w.title {
                t.ticks += 1;
            } else {
                t.title = w.title.clone();
                t.ticks = 1;
            }
            if t.ticks >= self.stable_ticks {
                t.stable = Some(w.title.clone());
            }
            // Устойчивость положения копится с первого такта, когда окно
            // видно, — независимо от того, устоялся ли уже заголовок. Слот
            // заводится позже, заголовком и сессией не ограничен: не копи
            // трекер это здесь, окну пришлось бы устаиваться дважды подряд —
            // сперва по заголовку, потом ещё раз по месту — прежде чем
            // положение вообще попадёт в слот.
            if let Some(b) = w.bounds {
                if t.pending == Some(b) {
                    t.pending_ticks += 1;
                } else {
                    t.pending = Some(b);
                    t.pending_ticks = 1;
                }
            }
            let Some(stable) = t.stable.clone() else { continue };
            if winners.get(w.title.as_str()) != Some(&w.id) {
                continue;
            }
            let key = strip_decoration(&stable);
            if let Some(sid) = index.get(&key) {
                t.session_id = Some(sid.id.clone());
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
            if w.focused && slot.focused_at_ms != now_ms {
                slot.focused_at_ms = now_ms;
                // Отметка взгляда живёт на диске — значит её смена и есть повод
                // файл переписать. Без этой строки перезапуск трекера показывал
                // бы человеку непрочитанным всё, на что он смотрел с прошлой
                // записи файла.
                self.dirty = true;
            }
            // `lastSeen` растёт каждый такт и поводом для записи не считается:
            // считался бы — файл писался бы с `fsync` раз в секунду вечно. На
            // диск он попадает попутно, когда файл переписывают по другой
            // причине, и этого достаточно: читает его один и тот же процесс.
            slot.last_seen_ms = now_ms;
            if slot.title != key {
                slot.title = key.clone();
                self.dirty = true;
            }
            if let Some(r) = index.get(&key) {
                if !r.cwd.is_empty() && slot.cwd != r.cwd {
                    slot.cwd = r.cwd.clone();
                    self.dirty = true;
                }
            }
            // Расстановка спрашивается до того, как слот примет нынешние
            // координаты: иначе он ответил бы «окно уже там, где нужно».
            // Слот без координат предложения не порождает — двигать некуда —
            // но окно его всё равно истратило: второго раза не будет.
            //
            // Предложение гасится внутри проверки `w.bounds`, а не раньше:
            // `None` там — платформа не ответила на этом такте, норма, а не
            // сбой (см. `Seen.bounds`). Погаси мы признак до этой проверки,
            // именно такой такт — окно только что появилось, платформа ещё не
            // назвала координаты — сжигал бы предложение впустую, и окно
            // молча оставалось бы там, где открылось.
            let mut just_placed = false;
            if t.awaiting_placement {
                if let Some(now_at) = w.bounds {
                    t.awaiting_placement = false;
                    if let Some(want) = slot.bounds {
                        if want != now_at {
                            self.placements.push((w.id, want));
                            just_placed = true;
                        }
                    }
                }
            }
            // Такт, на котором расстановка только что запрошена, ничего не
            // подтверждает: окно физически ещё стоит на прежнем (нынешнем для
            // такта) месте, и это устаревшее значение не должно затереть
            // память слота раньше, чем просьба дойдёт до платформы.
            if !just_placed && t.pending_ticks >= self.stable_ticks {
                if let Some(b) = t.pending {
                    if slot.bounds != Some(b) {
                        slot.bounds = Some(b);
                        self.dirty = true;
                    }
                }
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

    /// Какие окна поставить на место и куда. Пусто — ставить нечего.
    ///
    /// Список живёт один такт: он про окна, появившиеся именно сейчас.
    pub fn placements(&self) -> Vec<(u64, Bounds)> {
        self.placements.clone()
    }

    /// Менялись ли слоты с прошлого вопроса. Спрашивается перед записью файла:
    /// он пишется с `fsync`, и писать его на каждом такте — плата за то, что не
    /// изменилось.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    /// Слоты в том виде, в каком они уезжают на диск.
    pub fn slots_state(&self) -> BTreeMap<String, SlotState> {
        self.slots
            .iter()
            .map(|(sid, s)| {
                (
                    sid.clone(),
                    SlotState {
                        bounds: s.bounds,
                        title: s.title.clone(),
                        cwd: s.cwd.clone(),
                        last_seen_ms: s.last_seen_ms,
                        focused_at_ms: s.focused_at_ms,
                    },
                )
            })
            .collect()
    }

    /// Поднять слоты с диска. Зовётся один раз при старте, до первого такта.
    ///
    /// Устойчивость положения (`Tracked::pending`) не восстанавливается и не
    /// может: она копится по окнам, а окна прошлого запуска — чужие
    /// идентификаторы, этот запуск их не видел.
    pub fn load_slots(&mut self, slots: BTreeMap<String, SlotState>) {
        for (sid, s) in slots {
            self.slots.insert(
                sid,
                Slot {
                    focused_at_ms: s.focused_at_ms,
                    bounds: s.bounds,
                    title: s.title,
                    cwd: s.cwd,
                    last_seen_ms: s.last_seen_ms,
                },
            );
        }
    }

    /// Сессии, у которых окно открыто на этом такте. Из них снапшотер собирает
    /// состав раскладки.
    pub fn open_session_ids(&self) -> Vec<String> {
        self.bound.keys().cloned().collect()
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
        self.dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Bounds;
    use crate::state::SlotState;
    use std::collections::BTreeMap;

    const SID: &str = "aaaaaaaa-1111-2222-3333-444444444444";

    fn index(pairs: &[(&str, &str)]) -> BTreeMap<String, crate::index::SessionRef> {
        pairs
            .iter()
            .map(|(t, s)| {
                (t.to_string(), crate::index::SessionRef { id: s.to_string(), cwd: String::new() })
            })
            .collect()
    }

    fn seen(id: u64, title: &str) -> Seen {
        Seen { id, title: title.to_string(), focused: false, bounds: None }
    }

    fn seen_at(id: u64, title: &str, b: Bounds) -> Seen {
        Seen { id, title: title.to_string(), focused: false, bounds: Some(b) }
    }

    fn rect(x: i32, y: i32) -> Bounds {
        Bounds { x, y, width: 800, height: 600 }
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
        t.tick(&[Seen { id: 1, title: "ccfzf".into(), focused: true, bounds: None }], &idx, 5_000);
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
        t.tick(&[Seen { id: 1, title: "ccfzf".into(), focused: true, bounds: None }], &idx, 5_000);
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
        t.tick(&[Seen { id: 1, title: "ccfzf".into(), focused: true, bounds: None }], &idx, 5_000);
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
        t.tick(&[Seen { id: 1, title: "ccfzf".into(), focused: true, bounds: None }], &idx, 5_000);
        t.mark_unread(SID);
        t.tick(&[Seen { id: 1, title: "ccfzf".into(), focused: true, bounds: None }], &idx, 7_000);
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

    #[test]
    fn an_appearing_window_is_asked_to_go_back_where_it_was() {
        // Ради этого весь этап: сессию открыли заново, окно встаёт туда же.
        let mut t = Tracker::new(1);
        let idx = index(&[("ccfzf", SID)]);
        let mut slots = BTreeMap::new();
        slots.insert(SID.to_string(), SlotState {
            bounds: Some(rect(100, 100)), ..Default::default()
        });
        t.load_slots(slots);
        // Первый такт после запуска не ставит ничего — правило ниже.
        t.tick(&[], &idx, 1_000);
        t.tick(&[seen_at(1, "ccfzf", rect(700, 700))], &idx, 2_000);
        assert_eq!(t.placements(), vec![(1, rect(100, 100))]);
    }

    #[test]
    fn the_first_tick_after_start_places_nothing() {
        // Перезапуск трекера случается на каждой выкатке. Без этого правила он
        // сгребал бы все открытые окна по вчерашним местам — включая те, что
        // человек только что подвинул сам.
        let mut t = Tracker::new(1);
        let idx = index(&[("ccfzf", SID)]);
        let mut slots = BTreeMap::new();
        slots.insert(SID.to_string(), SlotState {
            bounds: Some(rect(100, 100)), ..Default::default()
        });
        t.load_slots(slots);
        t.tick(&[seen_at(1, "ccfzf", rect(700, 700))], &idx, 1_000);
        assert!(t.placements().is_empty(), "первый такт не расставляет");
    }

    #[test]
    fn a_window_already_in_place_is_not_touched() {
        // Просьба к платформе стоит вызова Accessibility, а он синхронный.
        // Двигать окно туда, где оно уже стоит, — это плата ни за что.
        let mut t = Tracker::new(1);
        let idx = index(&[("ccfzf", SID)]);
        let mut slots = BTreeMap::new();
        slots.insert(SID.to_string(), SlotState {
            bounds: Some(rect(100, 100)), ..Default::default()
        });
        t.load_slots(slots);
        t.tick(&[], &idx, 1_000);
        t.tick(&[seen_at(1, "ccfzf", rect(100, 100))], &idx, 2_000);
        assert!(t.placements().is_empty());
    }

    #[test]
    fn a_window_that_stayed_is_not_placed_again() {
        // Ставится появившееся окно, а не любое видимое. Иначе трекер воевал бы
        // с человеком за каждое перетаскивание — и победил бы трекер.
        let mut t = Tracker::new(1);
        let idx = index(&[("ccfzf", SID)]);
        let mut slots = BTreeMap::new();
        slots.insert(SID.to_string(), SlotState {
            bounds: Some(rect(100, 100)), ..Default::default()
        });
        t.load_slots(slots);
        t.tick(&[], &idx, 1_000);
        t.tick(&[seen_at(1, "ccfzf", rect(700, 700))], &idx, 2_000);
        assert_eq!(t.placements().len(), 1);
        t.tick(&[seen_at(1, "ccfzf", rect(700, 700))], &idx, 3_000);
        assert!(t.placements().is_empty(), "окно уже было в прошлом такте");
    }

    #[test]
    fn a_delayed_placement_leaves_the_remembered_bounds_untouched() {
        // Боевая настройка — stable_ticks = 2 (src-tauri/src/main.rs). Заголовок
        // окна устаивается не на том же такте, на котором оно появилось, а
        // слот находится только вместе с заголовком. Расстановка обязана всё
        // равно случиться — и не потерять память слота на том же такте: окно
        // физически ещё стоит на старом (для трекера — «нынешнем») месте, и
        // это устаревшее значение не должно затереть то, что мы только что
        // попросили вернуть.
        let mut t = Tracker::new(2);
        let idx = index(&[("ccfzf", SID)]);
        let mut slots = BTreeMap::new();
        slots.insert(SID.to_string(), SlotState {
            bounds: Some(rect(100, 100)), ..Default::default()
        });
        t.load_slots(slots);
        t.tick(&[], &idx, 1_000);
        // Такт 2: окно появилось, заголовку ещё не хватило тактов на устойчивость.
        t.tick(&[seen_at(1, "ccfzf", rect(700, 700))], &idx, 2_000);
        assert!(t.placements().is_empty(), "заголовок ещё не устоялся — слота не нашли");
        // Такт 3: заголовок устоялся, слот найден — вот когда уходит просьба.
        t.tick(&[seen_at(1, "ccfzf", rect(700, 700))], &idx, 3_000);
        assert_eq!(t.placements(), vec![(1, rect(100, 100))], "расстановка случилась ровно один раз");
        assert_eq!(
            t.slots_state()[SID].bounds,
            Some(rect(100, 100)),
            "координаты ещё не затёрты нынешним (устаревшим) положением окна",
        );
    }

    #[test]
    fn a_placement_is_offered_once_the_title_finally_resolves() {
        // Вход в сессию идёт через заголовок шелла: сперва он не значит
        // ничего для дампа, потом становится именем сессии. Слот находится не
        // на том такте, когда окно появилось, а на том, когда заголовок
        // наконец назвал сессию, — и предложение обязано дождаться именно
        // этого такта, а не сгореть раньше.
        let mut t = Tracker::new(1);
        let idx = index(&[("ccfzf", SID)]);
        let mut slots = BTreeMap::new();
        slots.insert(SID.to_string(), SlotState {
            bounds: Some(rect(100, 100)), ..Default::default()
        });
        t.load_slots(slots);
        t.tick(&[], &idx, 1_000);
        // Окно открылось с заголовком шелла — сессии под таким именем нет.
        t.tick(&[seen_at(1, "zsh", rect(700, 700))], &idx, 2_000);
        assert!(t.placements().is_empty(), "заголовок ещё не сессии — слота не нашли");
        // Заголовок сменился на имя сессии — вот когда предложение и уходит.
        t.tick(&[seen_at(1, "ccfzf", rect(700, 700))], &idx, 3_000);
        assert_eq!(t.placements(), vec![(1, rect(100, 100))]);
    }

    #[test]
    fn a_window_open_before_the_tracker_started_never_gets_an_offer() {
        // Правило 2 в новой форме: признак «предложение положено» живёт на
        // окне и не гаснет сам собой. Не запрети его такт запуска навсегда —
        // а лишь пропусти этот конкретный такт, — устоявшийся на такт позже
        // заголовок пробил бы в правиле дыру: окно, открытое до трекера,
        // получило бы расстановку, как только дамп наконец назвал его сессию.
        let mut t = Tracker::new(2);
        let idx = index(&[("ccfzf", SID)]);
        let mut slots = BTreeMap::new();
        slots.insert(SID.to_string(), SlotState {
            bounds: Some(rect(100, 100)), ..Default::default()
        });
        t.load_slots(slots);
        // Окно уже открыто на самом первом такте — трекер его не открывал.
        t.tick(&[seen_at(1, "ccfzf", rect(700, 700))], &idx, 1_000);
        t.tick(&[seen_at(1, "ccfzf", rect(700, 700))], &idx, 2_000);
        assert!(t.placements().is_empty(), "окно старше трекера — расстановки не будет никогда");
    }

    #[test]
    fn a_reopened_window_gets_its_own_placement_offer() {
        // Закрытое и вновь открытое окно — новый id, и своё предложение оно
        // получает заново: истраченный признак прошлого окна не должен
        // украсть его у следующего, даже когда оба ведут в один слот.
        let mut t = Tracker::new(1);
        let idx = index(&[("ccfzf", SID)]);
        let mut slots = BTreeMap::new();
        slots.insert(SID.to_string(), SlotState {
            bounds: Some(rect(100, 100)), ..Default::default()
        });
        t.load_slots(slots);
        t.tick(&[], &idx, 1_000);
        // Окно 1 появилось, получило и истратило предложение.
        t.tick(&[seen_at(1, "ccfzf", rect(700, 700))], &idx, 2_000);
        assert_eq!(t.placements(), vec![(1, rect(100, 100))]);
        // Окно закрылось.
        t.tick(&[], &idx, 3_000);
        // На его месте открылось новое — другой id, та же сессия по заголовку.
        t.tick(&[seen_at(2, "ccfzf", rect(900, 900))], &idx, 4_000);
        assert_eq!(t.placements(), vec![(2, rect(100, 100))], "новое окно — своё предложение");
    }

    #[test]
    fn a_tick_without_geometry_does_not_burn_the_placement_offer() {
        // `Seen.bounds == None` — платформа не ответила на этом такте, и это
        // норма, а не сбой (раздел «Отказы» спеки). Такой такт как раз и
        // выпадает на появление окна: заголовок только устоялся, а координаты
        // платформа ещё не вернула. Если бы признак «предложение положено»
        // гас раньше проверки `w.bounds`, окно молча осталось бы там, где
        // открылось, — отказ невоспроизводимый по требованию.
        let mut t = Tracker::new(1);
        let idx = index(&[("ccfzf", SID)]);
        let mut slots = BTreeMap::new();
        slots.insert(SID.to_string(), SlotState {
            bounds: Some(rect(100, 100)), ..Default::default()
        });
        t.load_slots(slots);
        t.tick(&[], &idx, 1_000);
        // Окно появилось, заголовок устоялся, но координат платформа не дала.
        t.tick(&[seen(1, "ccfzf")], &idx, 2_000);
        assert!(t.placements().is_empty(), "координат нет — решения о расстановке ещё не было");
        // На следующем такте платформа ответила — расстановка происходит,
        // предложение дождалось геометрии, а не сгорело тактом раньше.
        t.tick(&[seen_at(1, "ccfzf", rect(700, 700))], &idx, 3_000);
        assert_eq!(t.placements(), vec![(1, rect(100, 100))], "предложение дождалось геометрии");
    }

    #[test]
    fn a_session_seen_for_the_first_time_is_left_where_it_opened() {
        let mut t = Tracker::new(1);
        let idx = index(&[("ccfzf", SID)]);
        t.tick(&[], &idx, 1_000);
        t.tick(&[seen_at(1, "ccfzf", rect(700, 700))], &idx, 2_000);
        assert!(t.placements().is_empty(), "координат не помним — двигать некуда");
    }

    #[test]
    fn a_moved_window_is_remembered_only_after_it_settles() {
        // Пока окно тащат мышкой, координаты меняются каждый такт. Записывать
        // их немедленно значило бы звать fsync на каждый такт перетаскивания.
        let mut t = Tracker::new(2);
        let idx = index(&[("ccfzf", SID)]);
        t.tick(&[seen_at(1, "ccfzf", rect(0, 0))], &idx, 1_000);
        t.tick(&[seen_at(1, "ccfzf", rect(0, 0))], &idx, 2_000);
        assert_eq!(t.slots_state()[SID].bounds, Some(rect(0, 0)));
        // Потащили: два разных положения подряд, ни одно не устоялось.
        t.tick(&[seen_at(1, "ccfzf", rect(50, 0))], &idx, 3_000);
        t.tick(&[seen_at(1, "ccfzf", rect(120, 0))], &idx, 4_000);
        assert_eq!(t.slots_state()[SID].bounds, Some(rect(0, 0)), "на лету не запоминаем");
        // Отпустили: положение повторилось нужное число тактов.
        t.tick(&[seen_at(1, "ccfzf", rect(200, 0))], &idx, 5_000);
        t.tick(&[seen_at(1, "ccfzf", rect(200, 0))], &idx, 6_000);
        assert_eq!(t.slots_state()[SID].bounds, Some(rect(200, 0)), "устоялось — запомнили");
    }

    #[test]
    fn the_state_is_written_only_when_something_changed() {
        // Файл пишется с fsync. Писать его на каждом такте — плата за то, что
        // не изменилось.
        let mut t = Tracker::new(1);
        let idx = index(&[("ccfzf", SID)]);
        t.tick(&[seen_at(1, "ccfzf", rect(0, 0))], &idx, 1_000);
        assert!(t.take_dirty(), "первое появление слота — изменение");
        assert!(!t.take_dirty(), "спросили дважды — второй раз чисто");
        t.tick(&[seen_at(1, "ccfzf", rect(0, 0))], &idx, 2_000);
        assert!(!t.take_dirty(), "ничего не менялось");
        t.tick(&[seen_at(1, "ccfzf", rect(300, 300))], &idx, 3_000);
        assert!(t.take_dirty(), "координаты устоялись на новом месте");
    }

    #[test]
    fn the_working_directory_reaches_the_slot() {
        // Оттуда его возьмёт снимок.
        let mut t = Tracker::new(1);
        let mut idx = BTreeMap::new();
        idx.insert("ccfzf".to_string(), crate::index::SessionRef {
            id: SID.to_string(),
            cwd: "~/projects/js/ccfzf-picker".to_string(),
        });
        t.tick(&[seen_at(1, "ccfzf", rect(0, 0))], &idx, 1_000);
        assert_eq!(t.slots_state()[SID].cwd, "~/projects/js/ccfzf-picker");
    }

    #[test]
    fn a_loaded_focus_stamp_is_not_forgotten() {
        // Отметка взгляда переживает перезапуск: иначе после каждой выкатки
        // человек видел бы все сессии непрочитанными разом.
        let mut t = Tracker::new(1);
        let idx = index(&[("ccfzf", SID)]);
        let mut slots = BTreeMap::new();
        slots.insert(SID.to_string(), SlotState { focused_at_ms: 4_000, ..Default::default() });
        t.load_slots(slots);
        t.tick(&[seen_at(1, "ccfzf", rect(0, 0))], &idx, 9_000);
        assert_eq!(t.bound()[SID].focused_at_ms, 4_000);
    }

    #[test]
    fn open_sessions_are_listed_for_the_snapshotter() {
        let mut t = Tracker::new(1);
        let idx = index(&[("ccfzf", SID)]);
        t.tick(&[seen_at(1, "ccfzf", rect(0, 0))], &idx, 1_000);
        assert_eq!(t.open_session_ids(), vec![SID.to_string()]);
    }
}
