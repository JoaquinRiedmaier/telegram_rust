use dotenvy::dotenv;
use std::env; //Para archivo .env
use std::fs;
use std::process::Stdio;
use std::sync::OnceLock;
use teloxide::prelude::*;
use teloxide::repls::CommandReplExt;
use teloxide::utils::command::BotCommands; // Agora sim, le vamos a mandar mensajitos
use tokio::io::{AsyncBufReadExt, BufReader}; // Para leer línea por línea
use tokio::process::Command;
use tokio::time;

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
    inicio(bot.clone(), chat.clone()).await;
    tokio::spawn(bateria(bot.clone(), chat.clone()));
    tokio::spawn(ssh(bot.clone(), chat.clone()));
    MisComandos::repl(bot, answer).await;
    Ok(())
}

// Seccion mensajes
#[derive(BotCommands, Clone)]
#[command(
    rename_rule = "lowercase",
    description = "These commands are supported:"
)]
enum MisComandos {
    #[command(description = "mostrar estado del servidor")]
    Estado,
    #[command(description = "Parar syncthing")]
    PararSyncthing,
    #[command(description = "Arrancar syncthing")]
    ArrancarSyncthing,
    #[command(description = "Parar Tailscale")]
    Reiniciar,
    #[command(description = "Arrancar Tailscale")]
    ArrancarTailscale,
}

async fn answer(bot: Bot, msg: Message, cmd: MisComandos) -> ResponseResult<()> {
    let mut payload = String::from("Estado de servicios:\n");
    match cmd {
        MisComandos::Estado => {
            let status = chequeo_servicio("syncthing", &["is-active", get_syncthing_service()].as_slice()).await;
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
            let status = chequeo_servicio("syncthing", &["is-active", get_syncthing_service()].as_slice()).await;
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
            let child = Command::new("sudo")
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
                let child = Command::new("sudo")
                    .args(&["tailscale", "up"])
                    .status()
                    .await;
            } else {
                log::info!("Servicio ya está conectado.");
                // Mensaje a Telegram: Ya está corriendo
            }
        }
    };
    Ok(())
}
