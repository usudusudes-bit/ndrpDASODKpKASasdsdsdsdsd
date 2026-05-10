use rusqlite::{Connection, Result as SqlResult};
use tauri::State;
use bcrypt::{hash, verify, DEFAULT_COST};
use rand::{distributions::Alphanumeric, Rng};
use serde_json::json;
use log::{info, error};
use std::env;
use tokio::sync::Mutex;
use serde::{Deserialize, Serialize};

// Состояние приложения Tauri
struct AppState {
    db: Mutex<Connection>,
}

#[derive(Serialize, Deserialize)]
struct User {
    id: i32,
    username: String,
    email: String,
    created_at: String,
}

#[derive(Serialize, Deserialize)]
struct PromoCode {
    id: i32,
    code: String,
    uses_left: i32,
}

#[derive(Serialize, Deserialize)]
struct RconSettings {
    host: String,
    port: String,
    password: Option<String>,
}

#[tauri::command]
async fn register(
    state: State<'_, AppState>, 
    username: String, 
    email: String, 
    password: String
) -> Result<String, String> {
    let hashed_password = hash(&password, DEFAULT_COST).map_err(|e| format!("Ошибка хэширования: {}", e))?;

    let conn = state.db.lock().await;
    
    // Check if exists
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM users WHERE username = ?1 OR email = ?2)",
        [&username, &email],
        |row| row.get(0)
    ).unwrap_or(false);

    if exists {
        return Err("Пользователь с таким ником или email уже существует".to_string());
    }

    conn.execute(
        "INSERT INTO users (username, email, password, is_verified) VALUES (?1, ?2, ?3, 1)",
        (&username, &email, &hashed_password),
    ).map_err(|e| format!("Ошибка сохранения пользователя: {}", e))?;

    Ok(json!({
        "msg": "Регистрация успешна!",
        "username": username
    }).to_string())
}

#[tauri::command]
async fn login(state: State<'_, AppState>, identifier: String, password: String) -> Result<String, String> {
    let row_data = {
        let conn = state.db.lock().await;

        let is_email = identifier.contains('@');
        let query = if is_email {
            "SELECT username, password FROM users WHERE email = ?1"
        } else {
            "SELECT username, password FROM users WHERE username = ?1"
        };

        let mut stmt = conn.prepare(query).map_err(|e| e.to_string())?;
        let mut rows = stmt.query([&identifier]).map_err(|e| e.to_string())?;

        if let Some(row) = rows.next().map_err(|e| e.to_string())? {
            let actual_username: String = row.get(0).map_err(|e| e.to_string())?;
            let stored_hash: String = row.get(1).map_err(|e| e.to_string())?;
            Some((actual_username, stored_hash))
        } else {
            None
        }
    };

    if let Some((actual_username, stored_hash)) = row_data {
        let valid = verify(&password, &stored_hash).map_err(|e| e.to_string())?;
        
        if valid {
            Ok(json!({
                "msg": "Вход успешен",
                "username": actual_username
            }).to_string())
        } else {
            Err("Неверный пароль".to_string())
        }
    } else {
        Err("Пользователь не найден".to_string())
    }
}

#[tauri::command]
async fn get_users(state: State<'_, AppState>) -> Result<Vec<User>, String> {
    let conn = state.db.lock().await;
    let mut stmt = conn.prepare("SELECT id, username, email, created_at FROM users").map_err(|e| e.to_string())?;
    
    let users = stmt.query_map([], |row| {
        Ok(User {
            id: row.get(0)?,
            username: row.get(1)?,
            email: row.get(2)?,
            created_at: row.get(3)?,
        })
    }).map_err(|e| e.to_string())?
    .filter_map(Result::ok)
    .collect();

    Ok(users)
}

#[tauri::command]
async fn save_rcon(state: State<'_, AppState>, host: String, port: String, password: String) -> Result<String, String> {
    let conn = state.db.lock().await;
    conn.execute("DELETE FROM rcon_settings", []).map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO rcon_settings (host, port, password) VALUES (?1, ?2, ?3)",
        (&host, &port, &password),
    ).map_err(|e| e.to_string())?;
    Ok("RCON настройки сохранены".to_string())
}

#[tauri::command]
async fn get_rcon(state: State<'_, AppState>) -> Result<RconSettings, String> {
    let conn = state.db.lock().await;
    let mut stmt = conn.prepare("SELECT host, port, password FROM rcon_settings LIMIT 1").map_err(|e| e.to_string())?;
    let mut rows = stmt.query([]).map_err(|e| e.to_string())?;
    
    if let Some(row) = rows.next().map_err(|e| e.to_string())? {
        Ok(RconSettings {
            host: row.get(0).map_err(|e| e.to_string())?,
            port: row.get(1).map_err(|e| e.to_string())?,
            password: row.get(2).unwrap_or(None),
        })
    } else {
        Err("Настройки не найдены".to_string())
    }
}

#[tauri::command]
async fn rcon_online(state: State<'_, AppState>) -> Result<String, String> {
    let rcon = get_rcon(state).await?;
    let pass = rcon.password.unwrap_or_default();
    let address = format!("{}:{}", rcon.host, rcon.port);
    
    match rcon::Connection::builder().enable_minecraft_quirks(true).connect(&address, &pass).await {
        Ok(mut conn) => {
            match conn.cmd("list").await {
                Ok(response) => {
                    // response for list command is like "There are 2 of a max of 20 players online: player1, player2"
                    // we can just extract the number
                    if let Some(num_str) = response.split(" are ").nth(1).and_then(|s| s.split(" of").next()) {
                        Ok(num_str.to_string())
                    } else {
                        Ok("0".to_string())
                    }
                },
                Err(e) => Err(format!("Ошибка RCON команды: {}", e))
            }
        },
        Err(e) => Err(format!("Ошибка подключения RCON: {}", e))
    }
}

#[tauri::command]
async fn get_stats(state: State<'_, AppState>) -> Result<String, String> {
    let conn = state.db.lock().await;
    let users_count: i32 = conn.query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0)).unwrap_or(0);
    let purchases_count: i32 = conn.query_row("SELECT COUNT(*) FROM purchases", [], |row| row.get(0)).unwrap_or(0);
    
    Ok(json!({
        "users": users_count,
        "purchases": purchases_count
    }).to_string())
}

#[tauri::command]
async fn generate_promocode(state: State<'_, AppState>, uses: i32) -> Result<String, String> {
    let code: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(8)
        .map(char::from)
        .collect::<String>()
        .to_uppercase();
        
    let conn = state.db.lock().await;
    conn.execute(
        "INSERT INTO promocodes (code, uses_left) VALUES (?1, ?2)",
        (&code, &uses),
    ).map_err(|e| e.to_string())?;
    
    Ok(code)
}

#[tauri::command]
async fn get_promocodes(state: State<'_, AppState>) -> Result<Vec<PromoCode>, String> {
    let conn = state.db.lock().await;
    let mut stmt = conn.prepare("SELECT id, code, uses_left FROM promocodes").map_err(|e| e.to_string())?;
    
    let codes = stmt.query_map([], |row| {
        Ok(PromoCode {
            id: row.get(0)?,
            code: row.get(1)?,
            uses_left: row.get(2)?,
        })
    }).map_err(|e| e.to_string())?
    .filter_map(Result::ok)
    .collect();

    Ok(codes)
}

#[tauri::command]
async fn make_purchase(state: State<'_, AppState>, username: String, item: String, duration: String, price: String) -> Result<String, String> {
    let conn = state.db.lock().await;
    conn.execute(
        "INSERT INTO purchases (username, item, duration, price) VALUES (?1, ?2, ?3, ?4)",
        (&username, &item, &duration, &price),
    ).map_err(|e| e.to_string())?;
    
    Ok("Покупка успешно добавлена".to_string())
}

fn init_db() -> SqlResult<Connection> {
    let conn = Connection::open("hotland.db")?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS users (
            id INTEGER PRIMARY KEY,
            username TEXT NOT NULL UNIQUE,
            email TEXT NOT NULL UNIQUE,
            password TEXT NOT NULL,
            is_verified INTEGER DEFAULT 1,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP
        )",
        [],
    )?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS promocodes (
            id INTEGER PRIMARY KEY,
            code TEXT NOT NULL UNIQUE,
            uses_left INTEGER DEFAULT 1,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP
        )",
        [],
    )?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS rcon_settings (
            id INTEGER PRIMARY KEY,
            host TEXT NOT NULL,
            port TEXT NOT NULL,
            password TEXT
        )",
        [],
    )?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS purchases (
            id INTEGER PRIMARY KEY,
            username TEXT NOT NULL,
            item TEXT NOT NULL,
            duration TEXT NOT NULL,
            price TEXT NOT NULL,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP
        )",
        [],
    )?;

    Ok(conn)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    dotenvy::dotenv().ok();

    let conn = init_db().expect("Failed to initialize database");
    
    let app_state = AppState {
        db: Mutex::new(conn),
    };

    tauri::Builder::default()
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            register, login, get_users, save_rcon, get_rcon, rcon_online, get_stats, generate_promocode, get_promocodes, make_purchase
        ])
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
