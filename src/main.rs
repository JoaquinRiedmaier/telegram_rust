mod db;
mod reminders;

use dotenvy::dotenv;
use std::env; //Para archivo .env
use std::fs;
use std::process::Stdio;
use std::sync::{Arc, OnceLock};
use teloxide::prelude::*;
use teloxide::utils::command::BotCommands; // Agora sim, le vamos a mandar mensajitos
use tokio::io::{AsyncBufReadExt, BufReader}; // Para leer línea por línea
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::time;

/// Valida "HH:MM" en 24h sin depender de la crate `regex`.
fn validar_hora(hora: &str) -> bool {
    let bytes = hora.as_bytes();
    if bytes.len() != 5 || bytes[2] != b':' {
        return false;
    }
    let h: u8 = match hora[0..2].parse() {
        Ok(v) => v,
        Err(_) => return false,
    };
    let m: u8 = match hora[3..5].parse() {
        Ok(v) => v,
        Err(_) => return false,
    };
    h <= 23 && m <= 59
}

static SYNCTHING_SERVICE: OnceLock<String> = OnceLock::new();

fn get_syncthing_service() -> &'static str {
    SYNCTHING_SERVICE.get_or_init(|| {
        env::var("SYNCTHING").expect("Problema con la variable de entorno SYNCTHING")
    })
}

// Para cada servicio, ejecuta el comando y actualiza el payload
async fn chequeo_servicio(
    service_name: &str,
    command: &[&str],
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let mut flag_activo: bool = false;
    let mut status = String::new();
    let mut child = Command::new("systemctl")
        .args(command)
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("Error al ejecutar systemctl: {}", e))?;

    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout).lines();
    while let Ok(Some(line)) = reader.next_line().await {
        if line.contains("active") {
            status = format!("Corriendo {}\n", service_name);
            flag_activo = true;
            break;
        }
    }
    if !flag_activo {
        status = format!("Parado {}\n", service_name);
    }
    Ok(status)
}

async fn bateria(bot: Bot, chat: ChatId) {
    let mut estado_anterior = true;
    let ruta = "/sys/class/power_supply/AC/online";
    loop {
        let archivo = fs::read_to_string(ruta).expect("No se pudo leer el estado de la batería");
        let nueva_lectura = archivo.trim() == "1";
        if estado_anterior != nueva_lectura {
            estado_anterior = nueva_lectura;
            if nueva_lectura {
                bot.send_message(chat, "Volvio la luz")
                    .protect_content(true)
                    .await
                    .unwrap();
            } else {
                bot.send_message(chat, "Se cortó la luz")
                    .protect_content(true)
                    .await
                    .unwrap();
            }
        }
        time::sleep(time::Duration::from_secs(10)).await
    }
}

async fn ssh(bot: Bot, chat: ChatId) {
    let mut hijo = Command::new("journalctl")
        .args(["_COMM=sshd", "-f", "-n", "0"])
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("Fallo al iniciar el proceso journalctl");

    let stdout = hijo.stdout.take().expect("Fallo lectura de journalctl");
    let mut lector = BufReader::new(stdout).lines();
    while let Ok(Some(linea)) = lector.next_line().await {
        if linea.contains("Accepted publickey") || linea.contains("Accepted password") {
            log::info!("Detecta la desconexión SSH");
            if let Err(e) = bot
                .send_message(chat, format!("Nuevo inicio SSH:\n{}", linea))
                .protect_content(true)
                .await
            {
                log::error!("Error enviando alerta SSH: {}", e);
            }
        } else if linea.contains("Disconnected from user") {
            // Esto no anda
            log::info!("Detecta la desconexión SSH");
            if let Err(e) = bot
                .send_message(chat, "Desconexión SSH detectada.")
                .protect_content(true)
                .await
            {
                log::error!("Error enviando alerta SSH: {}", e);
            }
        }
    }
}

async fn inicio(bot: Bot, chat: ChatId) {
    // Solo cuando se inicia me avisa una vez
    bot.send_message(chat, "Tamo de volta papi")
        .protect_content(true)
        .await
        .ok();
    //Comando para leer el estado de syncthing
    let mut payload = String::from("Estado de servicios:\n");
    let status = chequeo_servicio(
        "syncthing",
        &["is-active", get_syncthing_service()].as_slice(),
    )
    .await;
    payload.push_str(&status.unwrap_or_default());
    //Tailscale
    let output = Command::new("tailscale")
        .arg("status")
        .output()
        .await
        .expect("Fallo al consultar tailscale");
    let status_str = String::from_utf8_lossy(&output.stdout);
    if status_str.contains("stopped") {
        payload.push_str("Tailscale detenido");
    } else {
        payload.push_str("Corriendo Tailscale");
    }
    if !payload.is_empty() {
        if let Err(e) = bot.send_message(chat, &payload).protect_content(true).await {
            log::error!("Error enviando alerta estado de servicios: {}", e);
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    //Lectura de variables de entorno
    dotenv().ok();
    let token = env::var("TB_TOKEN").expect("Problema con la variable de entorno TB_TOKEN");
    let chat = env::var("CHAT_ID").expect("Problema con la variable de entorno CHAT_ID");
    let chat = ChatId(
        chat.parse::<i64>()
            .expect("CHAT_ID debe ser un número entero"),
    ); //Casteamos a entero el chat id
    pretty_env_logger::init();
    log::info!("Iniciando bot...");
    let bot = Bot::new(token);

    // --- Inicializar base de datos de recordatorios ---
    let dir_bd = "/var/lib/BotLelegram";
    std::fs::create_dir_all(dir_bd).expect("No se pudo crear /var/lib/BotLelegram");
    let ruta_bd = format!("{}/recordatorios.db3", dir_bd);
    let conn_db = db::init_db(&ruta_bd).expect("No se pudo abrir la base de datos");
    let conn_db = Arc::new(Mutex::new(conn_db));

    inicio(bot.clone(), chat.clone()).await;

    let _ = bot
        .set_my_commands(MisComandos::bot_commands())
        .await
        .log_on_error();

    tokio::spawn(bateria(bot.clone(), chat.clone()));
    tokio::spawn(ssh(bot.clone(), chat.clone()));
    // --- Tarea de recordatorios (cada minuto) ---
    tokio::spawn(reminders::tarea_recordatorios(
        bot.clone(),
        chat,
        Arc::clone(&conn_db),
    ));

    // Pasamos la conexión de BD al handler de comandos via dptree
    let handler = dptree::entry().branch(
        Update::filter_message()
            .filter_command::<MisComandos>()
            .endpoint(
                |bot: Bot,
                 msg: Message,
                 cmd: MisComandos,
                 conn: Arc<Mutex<rusqlite::Connection>>| async move {
                    answer(bot, msg, cmd, conn).await
                },
            ),
    );

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![conn_db])
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;

    Ok(())
}

// Seccion mensajes
#[derive(BotCommands, Clone)]
#[command(
    rename_rule = "snake_case",
    description = "These commands are supported:"
)]
enum MisComandos {
    #[command(description = "mostrar estado del servidor")]
    Estado,
    #[command(description = "Parar syncthing")]
    PararSyncthing,
    #[command(description = "Arrancar syncthing")]
    ArrancarSyncthing,
    #[command(description = "Reiniciar servidor")]
    Reiniciar,
    #[command(description = "Arrancar Tailscale")]
    ArrancarTailscale,
    #[command(description = "Agregar recordatorio: /aggrec HH:MM mensaje")]
    Aggrec { hora_y_mensaje: String },
    #[command(description = "Listar recordatorios activos")]
    Listrec,
    #[command(description = "Eliminar recordatorio por id: /elimrec <id>")]
    Elimrec { id: String },
}

async fn answer(
    bot: Bot,
    msg: Message,
    cmd: MisComandos,
    conn: Arc<Mutex<rusqlite::Connection>>,
) -> ResponseResult<()> {
    let mut payload = String::from("Estado de servicios:\n");
    match cmd {
        MisComandos::Estado => {
            let status = chequeo_servicio(
                "syncthing",
                &["is-active", get_syncthing_service()].as_slice(),
            )
            .await;
            payload.push_str(&status.unwrap_or_default());
            let output = Command::new("tailscale")
                .arg("status")
                .output()
                .await
                .expect("Fallo al consultar tailscale");
            let status_str = String::from_utf8_lossy(&output.stdout);
            if status_str.contains("stopped") {
                payload.push_str("Tailscale detenido");
            } else {
                payload.push_str("Corriendo Tailscale");
            }
            if !payload.is_empty() {
                if let Err(e) = bot
                    .send_message(msg.chat.id, &payload)
                    .protect_content(true)
                    .await
                {
                    log::error!("Error enviando alerta estado de servicios: {}", e);
                }
            }
        }
        MisComandos::PararSyncthing => {
            let status = chequeo_servicio(
                "syncthing",
                &["is-active", get_syncthing_service()].as_slice(),
            )
            .await;
            match status {
                Ok(status) => {
                    if status.contains("Corriendo") {
                        log::info!("Detecta que corre");
                        // Detener el servicio
                        let child = Command::new("sudo")
                            .args(&["systemctl", "stop", get_syncthing_service()])
                            .stdout(Stdio::piped())
                            .status()
                            .await
                            .map_err(|e| format!("Error al ejecutar systemctl: {}", e));
                        log::info!("Servicio detenido: {:?}", child);
                    } else {
                        log::info!("El servicio no está corriendo.");
                    }
                }
                Err(e) => {
                    log::info!("Consulta falla");
                    eprintln!("Error al verificar el estado del servicio: {}", e);
                }
            }
        }
        MisComandos::ArrancarSyncthing => {
            let status = chequeo_servicio(
                "syncthing",
                &["is-active", get_syncthing_service()].as_slice(),
            )
            .await;
            match status {
                Ok(status) => {
                    if status.contains("Parado") {
                        // Arrancar el servicio
                        let child = Command::new("sudo")
                            .args(&["systemctl", "start", get_syncthing_service()])
                            .stdout(Stdio::piped())
                            .status()
                            .await
                            .map_err(|e| format!("Error al ejecutar systemctl: {}", e));
                        log::info!("Servicio arrancado: {:?}", child);
                    } else {
                        log::info!("Servicio ya está corriendo: {:?}", status);
                    }
                }
                Err(e) => {
                    log::info!("Consulta falla");
                    eprintln!("Error al verificar el estado del servicio: {}", e);
                }
            }
        }
        MisComandos::Reiniciar => {
            let _child = Command::new("sudo")
                .args(&["reboot"])
                .stdout(Stdio::piped())
                .status()
                .await
                .map_err(|e| format!("Error al ejecutar systemctl: {}", e));
        }
        MisComandos::ArrancarTailscale => {
            // Consulta estado
            let output = Command::new("tailscale")
                .arg("status")
                .output()
                .await
                .expect("Fallo al consultar tailscale");

            let status_str = String::from_utf8_lossy(&output.stdout);

            // arranca si esta detenido
            if status_str.contains("stopped") {
                log::info!("Tailscale apagado. Arrancando...");
                let _child = Command::new("sudo")
                    .args(&["tailscale", "up"])
                    .status()
                    .await;
            } else {
                log::info!("Servicio ya está conectado.");
            }
        }

        // ---- Recordatorios ----
        MisComandos::Aggrec { hora_y_mensaje } => {
            let partes: Vec<&str> = hora_y_mensaje.trim().splitn(2, ' ').collect();
            if partes.len() < 2 || partes[1].trim().is_empty() {
                bot.send_message(
                    msg.chat.id,
                    "❌ Uso: /aggrec HH:MM mensaje\nEjemplo: /aggrec 14:30 Reunión con Juan",
                )
                .protect_content(true)
                .await?;
            } else {
                let hora = partes[0].to_string();
                let mensaje = partes[1].trim().to_string();
                if !validar_hora(&hora) {
                    bot.send_message(
                        msg.chat.id,
                        "❌ Hora inválida. Usá el formato HH:MM en 24h (ej: 08:30, 14:00).",
                    )
                    .protect_content(true)
                    .await?;
                } else {
                    // Operación de BD: obtener resultado y soltar el guard antes del .await
                    let resultado = {
                        let guard = conn.lock().await;
                        db::agregar(&guard, &hora, &mensaje)
                    };
                    match resultado {
                        Ok(id) => {
                            bot.send_message(
                                msg.chat.id,
                                format!("✅ Recordatorio agregado (id: {}) — se disparará todos los días a las {} UTC-3.", id, hora),
                            )
                            .protect_content(true)
                            .await?;
                        }
                        Err(e) => {
                            log::error!("Error guardando recordatorio: {}", e);
                            bot.send_message(msg.chat.id, "❌ Error al guardar el recordatorio.")
                                .protect_content(true)
                                .await?;
                        }
                    }
                }
            }
        }

        MisComandos::Listrec => {
            // Soltar el guard antes del .await
            let resultado = {
                let guard = conn.lock().await;
                db::listar(&guard)
            };
            match resultado {
                Ok(lista) if lista.is_empty() => {
                    bot.send_message(msg.chat.id, "📭 No hay recordatorios activos.")
                        .protect_content(true)
                        .await?;
                }
                Ok(lista) => {
                    let mut texto = String::from("📋 Recordatorios activos:\n");
                    for rec in &lista {
                        texto.push_str(&format!("  #{} {} — {}\n", rec.id, rec.hora, rec.mensaje));
                    }
                    bot.send_message(msg.chat.id, texto)
                        .protect_content(true)
                        .await?;
                }
                Err(e) => {
                    log::error!("Error listando recordatorios: {}", e);
                    bot.send_message(msg.chat.id, "❌ Error al leer los recordatorios.")
                        .protect_content(true)
                        .await?;
                }
            }
        }

        MisComandos::Elimrec { id } => {
            match id.trim().parse::<i64>() {
                Err(_) => {
                    bot.send_message(
                        msg.chat.id,
                        "❌ ID inválido. Usá un número entero: /elimrec 3",
                    )
                    .protect_content(true)
                    .await?;
                }
                Ok(id_num) => {
                    // Soltar el guard antes del .await
                    let resultado = {
                        let guard = conn.lock().await;
                        db::eliminar(&guard, id_num)
                    };
                    match resultado {
                        Ok(true) => {
                            bot.send_message(
                                msg.chat.id,
                                format!("🗑️ Recordatorio #{} eliminado.", id_num),
                            )
                            .protect_content(true)
                            .await?;
                        }
                        Ok(false) => {
                            bot.send_message(
                                msg.chat.id,
                                format!("❌ No existe un recordatorio con id {}.", id_num),
                            )
                            .protect_content(true)
                            .await?;
                        }
                        Err(e) => {
                            log::error!("Error eliminando recordatorio {}: {}", id_num, e);
                            bot.send_message(msg.chat.id, "❌ Error al eliminar el recordatorio.")
                                .protect_content(true)
                                .await?;
                        }
                    }
                }
            }
        }
    };
    Ok(())
}
