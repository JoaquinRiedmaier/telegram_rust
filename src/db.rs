use rusqlite::{Connection, Result, params};

/// Representa un recordatorio almacenado en la base de datos.
pub struct Recordatorio {
    pub id: i64,
    pub hora: String,  // formato "HH:MM" en UTC-3
    pub mensaje: String,
}

/// Abre (o crea) la base de datos en la ruta indicada y crea la tabla si no existe.
/// El directorio padre debe existir antes de llamar esta función.
pub fn init_db(ruta: &str) -> Result<Connection> {
    let conn = Connection::open(ruta)?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS recordatorios (
            id      INTEGER PRIMARY KEY AUTOINCREMENT,
            hora    TEXT    NOT NULL,
            mensaje TEXT    NOT NULL
        );",
    )?;
    Ok(conn)
}

/// Inserta un nuevo recordatorio. Retorna el id asignado por la BD.
pub fn agregar(conn: &Connection, hora: &str, mensaje: &str) -> Result<i64> {
    conn.execute(
        "INSERT INTO recordatorios (hora, mensaje) VALUES (?1, ?2)",
        params![hora, mensaje],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Retorna todos los recordatorios activos, ordenados por hora.
pub fn listar(conn: &Connection) -> Result<Vec<Recordatorio>> {
    let mut stmt = conn.prepare(
        "SELECT id, hora, mensaje FROM recordatorios ORDER BY hora",
    )?;
    let recordatorios = stmt.query_map([], |row| {
        Ok(Recordatorio {
            id: row.get(0)?,
            hora: row.get(1)?,
            mensaje: row.get(2)?,
        })
    })?
    .collect::<Result<Vec<_>>>()?;
    Ok(recordatorios)
}

/// Elimina un recordatorio por su id. Retorna true si existia, false si no.
pub fn eliminar(conn: &Connection, id: i64) -> Result<bool> {
    let filas = conn.execute(
        "DELETE FROM recordatorios WHERE id = ?1",
        params![id],
    )?;
    Ok(filas > 0)
}

/// Retorna todos los recordatorios cuya hora coincide exactamente con `hora_actual`.
/// Se usa para disparar los recordatorios del minuto actual.
pub fn obtener_vencidos(conn: &Connection, hora_actual: &str) -> Result<Vec<Recordatorio>> {
    let mut stmt = conn.prepare(
        "SELECT id, hora, mensaje FROM recordatorios WHERE hora = ?1",
    )?;
    let recordatorios = stmt.query_map(params![hora_actual], |row| {
        Ok(Recordatorio {
            id: row.get(0)?,
            hora: row.get(1)?,
            mensaje: row.get(2)?,
        })
    })?
    .collect::<Result<Vec<_>>>()?;
    Ok(recordatorios)
}
