import express from 'express';
import cors from 'cors';
import Database from 'better-sqlite3';
import bcrypt from 'bcrypt';
import { Rcon } from 'rcon-client';
import path from 'path';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const app = express();
app.use(cors());
app.use(express.json());

// Инициализация базы данных
const db = new Database('hotland.db');

db.exec(`
  CREATE TABLE IF NOT EXISTS users (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      username TEXT NOT NULL UNIQUE,
      email TEXT NOT NULL UNIQUE,
      password TEXT NOT NULL,
      is_verified INTEGER DEFAULT 1,
      created_at DATETIME DEFAULT CURRENT_TIMESTAMP
  );

  CREATE TABLE IF NOT EXISTS promocodes (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      code TEXT NOT NULL UNIQUE,
      uses_left INTEGER DEFAULT 1,
      created_at DATETIME DEFAULT CURRENT_TIMESTAMP
  );

  CREATE TABLE IF NOT EXISTS rcon_settings (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      host TEXT NOT NULL,
      port TEXT NOT NULL,
      password TEXT
  );

  CREATE TABLE IF NOT EXISTS purchases (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      username TEXT NOT NULL,
      item TEXT NOT NULL,
      duration TEXT NOT NULL,
      price TEXT NOT NULL,
      created_at DATETIME DEFAULT CURRENT_TIMESTAMP
  );
`);

const handleReq = (handler) => async (req, res) => {
  try {
    const result = await handler(req.body);
    res.json({ result });
  } catch (err) {
    res.status(400).json({ error: err.message || err });
  }
};

// --- API Роуты ---

app.post('/api/register', handleReq(async ({ username, email, password }) => {
  const exists = db.prepare('SELECT 1 FROM users WHERE username = ? OR email = ?').get(username, email);
  if (exists) throw new Error('Пользователь с таким ником или email уже существует');

  const hashed = await bcrypt.hash(password, 10);
  db.prepare('INSERT INTO users (username, email, password) VALUES (?, ?, ?)').run(username, email, hashed);
  return { msg: 'Регистрация успешна!', username };
}));

app.post('/api/login', handleReq(async ({ identifier, password }) => {
  const isEmail = identifier.includes('@');
  const query = isEmail ? 'SELECT username, password FROM users WHERE email = ?' : 'SELECT username, password FROM users WHERE username = ?';
  const user = db.prepare(query).get(identifier);

  if (!user) throw new Error('Пользователь не найден');
  
  const valid = await bcrypt.compare(password, user.password);
  if (!valid) throw new Error('Неверный пароль');

  return JSON.stringify({ msg: 'Вход успешен', username: user.username });
}));

app.post('/api/get_users', handleReq(() => {
  return db.prepare('SELECT id, username, email, created_at FROM users').all();
}));

app.post('/api/save_rcon', handleReq(({ host, port, password }) => {
  db.prepare('DELETE FROM rcon_settings').run();
  db.prepare('INSERT INTO rcon_settings (host, port, password) VALUES (?, ?, ?)').run(host, port, password);
  return 'RCON настройки сохранены';
}));

app.post('/api/get_rcon', handleReq(() => {
  return db.prepare('SELECT host, port, password FROM rcon_settings LIMIT 1').get() || { host: '', port: '', password: '' };
}));

app.post('/api/rcon_online', handleReq(async () => {
  const rcon = db.prepare('SELECT host, port, password FROM rcon_settings LIMIT 1').get();
  if (!rcon || !rcon.host) return "0";

  try {
    const rconClient = await Rcon.connect({
      host: rcon.host,
      port: parseInt(rcon.port),
      password: rcon.password
    });
    
    const response = await rconClient.send("list");
    rconClient.end();
    
    const match = response.match(/There are (\d+) of a max/);
    return match ? match[1] : "0";
  } catch (e) {
    return "0";
  }
}));

app.post('/api/get_stats', handleReq(() => {
  const users = db.prepare('SELECT COUNT(*) as count FROM users').get().count;
  const purchases = db.prepare('SELECT COUNT(*) as count FROM purchases').get().count;
  return JSON.stringify({ users, purchases });
}));

app.post('/api/generate_promocode', handleReq(({ uses }) => {
  const code = Math.random().toString(36).substring(2, 10).toUpperCase();
  db.prepare('INSERT INTO promocodes (code, uses_left) VALUES (?, ?)').run(code, uses);
  return code;
}));

app.post('/api/get_promocodes', handleReq(() => {
  return db.prepare('SELECT id, code, uses_left FROM promocodes').all();
}));

app.post('/api/make_purchase', handleReq(({ username, item, duration, price }) => {
  db.prepare('INSERT INTO purchases (username, item, duration, price) VALUES (?, ?, ?, ?)').run(username, item, duration, price);
  return 'Покупка успешно добавлена';
}));

// --- РАЗДАЧА ФРОНТЕНДА (ОБЯЗАТЕЛЬНО ДЛЯ RENDER) ---

// Указываем папку 'dist' для статических файлов
app.use(express.static(path.join(__dirname, 'dist')));

// Для любых других запросов (React Router) отдаем index.html
app.get('*', (req, res) => {
  res.sendFile(path.join(__dirname, 'dist', 'index.html'));
});

// ПОРТ должен быть динамическим
const PORT = process.env.PORT || 3001;
app.listen(PORT, () => {
  console.log(`Server is running on port ${PORT}`);
});
