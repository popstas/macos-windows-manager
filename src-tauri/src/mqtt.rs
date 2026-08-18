//! Подписка на просьбы: поднять окно, вернуть в непрочитанное.
//!
//! Слушает сам трей, а не пикер и не команда по ssh. Причин три, и все три
//! оплачены на выкатке первого этапа: ssh-сессия живёт в фоновом домене
//! безопасности и до графической не достаёт; подъём из пикера стоил бы второго
//! разрешения Accessibility на бинарь, который пересобирается чаще; а реестр
//! окон (`AXUIElement`) не `Send` и живёт в потоке трекера.
//!
//! Отсюда и форма: свой поток на соединение, канал наружу, исполняет просьбу
//! поток трекера.

use mwm_core::config::MqttConfig;
use mwm_core::mwm_log;
use mwm_core::request::{command_from_topic, parse_request, Request};
use rumqttc::{Client, Event, MqttOptions, Packet, QoS};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

/// Пауза перед новой попыткой соединения. Брокер может лежать сколько угодно,
/// а трекер обязан продолжать публиковать окна: без паузы отказ соединения
/// крутился бы в горячем цикле и съел бы такт.
const RETRY: Duration = Duration::from_secs(5);

pub struct Link {
    live: Arc<AtomicBool>,
}

impl Link {
    /// Установлено ли соединение прямо сейчас.
    ///
    /// По этому ответу трекер объявляет `focus` в файле окон. Объявить умение
    /// поднимать окно, не имея транспорта, значит подарить человеку молчащий
    /// Enter — а это хуже открытого терминала.
    pub fn is_live(&self) -> bool {
        self.live.load(Ordering::Relaxed)
    }
}

/// Поднять подписку. Ненастроенный брокер — не отказ: канал просто молчит, а
/// `is_live()` всегда отвечает «нет».
///
/// Канал заводит не этот модуль: тот же конец нужен пунктам меню трея, а те
/// живут на главном потоке. Отсюда и `tx` в аргументах — подписка лишь один из
/// двух источников просьб.
pub fn spawn(cfg: &MqttConfig, tx: Sender<Request>) -> Link {
    let live = Arc::new(AtomicBool::new(false));
    if !cfg.is_configured() {
        return Link { live };
    }
    let worker_live = live.clone();
    let cfg = cfg.clone();
    std::thread::spawn(move || run(cfg, tx, worker_live));
    Link { live }
}

fn run(cfg: MqttConfig, tx: Sender<Request>, live: Arc<AtomicBool>) {
    let base = cfg.base.clone();
    let filter = format!("{base}/#");
    loop {
        let mut opts = MqttOptions::new(format!("mwm-{}", std::process::id()), &cfg.host, cfg.port);
        opts.set_keep_alive(Duration::from_secs(30));
        if !cfg.user.is_empty() {
            opts.set_credentials(&cfg.user, &cfg.password);
        }
        let (client, mut connection) = Client::new(opts, 16);
        if let Err(e) = client.subscribe(&filter, QoS::AtMostOnce) {
            mwm_log!("subscribe failed: {e}");
            live.store(false, Ordering::Relaxed);
            std::thread::sleep(RETRY);
            continue;
        }
        for event in connection.iter() {
            match event {
                // Живым соединение считается с подтверждения брокера, а не с
                // вызова `Client::new`: тот возвращает клиент сразу, до всякой
                // сети, и по нему трекер объявил бы умение, которого нет.
                Ok(Event::Incoming(Packet::ConnAck(_))) => {
                    live.store(true, Ordering::Relaxed);
                }
                Ok(Event::Incoming(Packet::Publish(p))) => {
                    let payload = String::from_utf8_lossy(&p.payload).to_string();
                    let Some(command) = command_from_topic(&p.topic, &base) else { continue };
                    // Незнакомая команда — молчание: на своей базе может
                    // оказаться что угодно, и жалоба на каждое сообщение забила
                    // бы журнал.
                    let Some(req) = parse_request(command, &payload) else { continue };
                    if tx.send(req).is_err() {
                        // Читателя не стало — трекер остановлен, и держать
                        // соединение больше не для кого.
                        return;
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    mwm_log!("mqtt connection lost: {e}");
                    live.store(false, Ordering::Relaxed);
                    break;
                }
            }
        }
        live.store(false, Ordering::Relaxed);
        std::thread::sleep(RETRY);
    }
}
