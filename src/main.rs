use dotenvy::dotenv;
use std::env; //Para archivo .env
use std::fs;
use teloxide::prelude::*;
use tokio::time;

// Estado de carga $ nano /sys/class/power_supply/AC/online
//

async fn bateria(bot: Bot, chat: ChatId) {
    let mut estado_anterior = true;
    let ruta = "/sys/class/power_supply/AC/online";
    loop {
        let archivo = fs::read_to_string(ruta).expect("No se pudo leer el estado de la batería");
        let nueva_lectura = archivo.trim() == "1";
        if estado_anterior != nueva_lectura {
            estado_anterior = nueva_lectura;
            if nueva_lectura {
                bot.send_message(chat, "Volvio la luz").await.unwrap();
            } else {
                bot.send_message(chat, "Se cortó la luz").await.unwrap();
            }
        }
        time::sleep(time::Duration::from_secs(10)).await
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

    bot.send_message(chat, "El bot ha iniciado correctamente")
        .await?;
    tokio::spawn(bateria(bot.clone(), chat.clone()));
    std::future::pending::<()>().await; // Provisorio para el ciclo
    Ok(())
}
