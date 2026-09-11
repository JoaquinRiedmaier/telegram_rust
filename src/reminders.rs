use std::sync::Arc;
use tokio::sync::Mutex;
use teloxide::prelude::*;
use tokio::time;

use crate::db;

const MENSAJE_8AM: &str = "⏰ Son las 8 de la mañana. ¡A comenzar el día!";

/// Obtiene la hora actual en UTC-3 con formato "HH:MM".
fn hora_utc3() -> String {
    let utm3 = chrono::FixedOffset::west_opt(3 * 3600).unwrap();
    let ahora = chrono::Local::now().with_timezone(&utm3);
    ahora.format("%H:%M").to_string()
}

/// Tarea de background. Cada 60 segundos:
///   1. Envía el mensaje fijo si son las 08:00 UTC-3.
///   2. Consulta la BD y envía los recordatorios cuya hora coincida.
///      Los recordatorios nunca se eliminan automáticamente.
pub async fn tarea_recordatorios(
    bot: Bot,
    chat_id: ChatId,
    conn: Arc<Mutex<rusqlite::Connection>>,
) {
    // Rastrea el último minuto en que se disparó el aviso de 08:00
    // para no repetirlo si el loop tarda menos de 60s.
    let mut ultimo_8am = String::new();

    loop {
        time::sleep(time::Duration::from_secs(60)).await;

        let hora = hora_utc3();

        // --- Recordatorio fijo 08:00 ---
        if hora == "08:00" && ultimo_8am != "08:00" {
            ultimo_8am = "08:00".to_string();
            if let Err(e) = bot
                .send_message(chat_id, MENSAJE_8AM)
                .protect_content(true)
                .await
            {
                log::error!("Error enviando recordatorio 08:00: {}", e);
            }
        } else if hora != "08:00" {
            ultimo_8am.clear();
        }

        // --- Recordatorios variables de la BD ---
        // Soltar el guard antes del .await
        let vencidos = {
            let guard = conn.lock().await;
            db::obtener_vencidos(&guard, &hora).unwrap_or_default()
        };

        for rec in vencidos {
            if let Err(e) = bot
                .send_message(chat_id, &rec.mensaje)
                .protect_content(true)
                .await
            {
                log::error!("Error enviando recordatorio id={}: {}", rec.id, e);
            }
        }
    }
}
