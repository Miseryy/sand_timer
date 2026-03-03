use anyhow::Result;
use chrono::Local;
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use rand::RngExt;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    widgets::{
        canvas::{Canvas, Context, Points},
        Block, Borders, Paragraph,
    },
    Terminal,
};
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const WIDTH: i32 = 100;
const HEIGHT: i32 = 160;

#[derive(Clone, Copy, PartialEq)]
enum Cell {
    Empty,
    Sand,
    Wall,
}

#[derive(PartialEq)]
enum AppState {
    Setting,
    Running,
    Paused,
    Finished,
}

struct App {
    grid: Vec<Vec<Cell>>,
    initial_sand: usize,
    physics_accumulator: Duration,
    physics_interval: Duration,
    gate_allowance: f64,
    grains_per_sec: f64,
    hole_radius: i32,
    last_tick: Instant,
    elapsed: Duration,
    h: u32,
    m: u32,
    s: u32,
    selected_unit: usize,
    state: AppState,
    wall_friction: f32,
    sand_friction: f32,
    is_24h: bool,
}

impl App {
    fn new(h: u32, m: u32, s: u32, wall_f: f32, sand_f: f32, is_24h: bool) -> Self {
        let mut grid = vec![vec![Cell::Empty; WIDTH as usize]; HEIGHT as usize];
        let center_x = WIDTH / 2;
        let center_y = HEIGHT / 2;

        let total_secs = (h * 3600 + m * 60 + s).max(1);

        let sand_start_y = center_y + 1;
        let sand_end_y = HEIGHT - 15;

        let mut estimated_sand = 0.0;
        for y in sand_start_y..sand_end_y {
            let dy = (y - center_y).abs();
            for x in 2..(WIDTH - 2) {
                let dx = (x - center_x).abs();
                if dx as f32 <= (dy as f32 / 1.1) {
                    estimated_sand += 1.0;
                }
            }
        }

        let estimated_gps = estimated_sand / total_secs as f64;
        let required_width = (estimated_gps / 30.0).ceil() as i32;
        let hole_radius = (required_width / 2).max(0);

        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let dx = (x - center_x).abs();
                let dy = (y - center_y).abs();
                if x < 2 || x >= WIDTH - 2 || y < 4 || y >= HEIGHT - 4 {
                    grid[y as usize][x as usize] = Cell::Wall;
                    continue;
                }
                if dy == 0 {
                    if dx > hole_radius {
                        grid[y as usize][x as usize] = Cell::Wall;
                    }
                } else {
                    if dx as f32 > (dy as f32 / 1.1) + hole_radius as f32 {
                        grid[y as usize][x as usize] = Cell::Wall;
                    }
                }
            }
        }

        let mut count = 0;
        for y in sand_start_y..sand_end_y {
            for d in 0..WIDTH / 2 {
                for sign in [-1, 1] {
                    let x = center_x + (d * sign);
                    if x >= 0 && x < WIDTH && grid[y as usize][x as usize] == Cell::Empty {
                        grid[y as usize][x as usize] = Cell::Sand;
                        count += 1;
                    }
                    if d == 0 {
                        break;
                    }
                }
            }
        }

        let grains_per_sec = count as f64 / total_secs as f64;
        let physics_interval = Duration::from_millis(16);

        Self {
            grid,
            initial_sand: count,
            physics_accumulator: Duration::ZERO,
            physics_interval,
            gate_allowance: 0.0,
            grains_per_sec,
            hole_radius,
            last_tick: Instant::now(),
            elapsed: Duration::ZERO,
            h,
            m,
            s,
            selected_unit: 2,
            state: AppState::Setting,
            wall_friction: wall_f,
            sand_friction: sand_f,
            is_24h,
        }
    }

    fn update(&mut self) {
        if self.state == AppState::Setting || self.state == AppState::Paused {
            self.last_tick = Instant::now();
            return;
        }

        let now = Instant::now();
        let delta = now.duration_since(self.last_tick);
        self.last_tick = now;
        self.physics_accumulator += delta;

        if self.state == AppState::Running {
            self.elapsed += delta;
            self.gate_allowance += self.grains_per_sec * delta.as_secs_f64();
        }

        if self.physics_accumulator >= self.physics_interval {
            self.physics_accumulator -= self.physics_interval;
            self.step_physics();

            if self.physics_accumulator > Duration::from_millis(50) {
                self.physics_accumulator = Duration::ZERO;
            }
        }
    }

    fn step_physics(&mut self) {
        let mut rng = rand::rng();
        let mut next_grid = vec![vec![Cell::Empty; WIDTH as usize]; HEIGHT as usize];
        for y in 0..HEIGHT as usize {
            for x in 0..WIDTH as usize {
                if self.grid[y][x] == Cell::Wall {
                    next_grid[y][x] = Cell::Wall;
                }
            }
        }

        let prim_dy = -1i32;
        let prim_dx = 0i32;
        let slide_a = (1i32, -1i32);
        let slide_b = (-1i32, -1i32);

        let center_x = (WIDTH / 2) as usize;
        let center_y = (HEIGHT / 2) as usize;

        let x_rev = rng.random_bool(0.5);

        for y in 0..HEIGHT as usize {
            for j in 0..WIDTH as usize {
                let x = if x_rev { WIDTH as usize - 1 - j } else { j };

                if self.grid[y][x] == Cell::Sand {
                    let mut moved = false;
                    let ny = y as i32 + prim_dy;
                    let nx = x as i32 + prim_dx;

                    if ny >= 0 && ny < HEIGHT as i32 && nx >= 0 && nx < WIDTH as i32 {
                        let ny = ny as usize;
                        let nx = nx as usize;
                        let is_at_gate =
                            (x as i32 - center_x as i32).abs() <= self.hole_radius && y == center_y;
                        let allowed = !is_at_gate
                            || self.gate_allowance >= 1.0
                            || self.state == AppState::Finished;

                        if allowed && next_grid[ny][nx] == Cell::Empty {
                            next_grid[ny][nx] = Cell::Sand;
                            moved = true;
                            if is_at_gate && self.state == AppState::Running {
                                self.gate_allowance -= 1.0;
                            }
                        } else if !is_at_gate {
                            let slides = if rng.random_bool(0.5) {
                                [slide_a, slide_b]
                            } else {
                                [slide_b, slide_a]
                            };
                            let is_in_source = y > center_y;
                            let adj_to_wall = (x > 0 && self.grid[y][x - 1] == Cell::Wall)
                                || (x + 1 < WIDTH as usize && self.grid[y][x + 1] == Cell::Wall)
                                || (y > 0 && self.grid[y - 1][x] == Cell::Wall)
                                || (y + 1 < HEIGHT as usize && self.grid[y + 1][x] == Cell::Wall);
                            let base_friction = if is_in_source {
                                self.sand_friction * 0.5
                            } else {
                                self.sand_friction
                            };
                            let friction = if adj_to_wall {
                                self.wall_friction
                            } else {
                                base_friction
                            };

                            if !rng.random_bool(friction as f64) {
                                for &(sdx, sdy) in &slides {
                                    let sy = y as i32 + sdy;
                                    let sx = x as i32 + sdx;
                                    if sy >= 0 && sy < HEIGHT as i32 && sx >= 0 && sx < WIDTH as i32
                                    {
                                        let (sy, sx) = (sy as usize, sx as usize);
                                        if next_grid[sy][sx] == Cell::Empty {
                                            next_grid[sy][sx] = Cell::Sand;
                                            moved = true;
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if !moved {
                        if next_grid[y][x] == Cell::Empty {
                            next_grid[y][x] = Cell::Sand;
                        } else {
                            let is_in_source = y > center_y;
                            let vibrate_chance = if is_in_source { 0.2 } else { 0.05 };
                            if rng.random_bool(vibrate_chance) {
                                let dxs = if rng.random_bool(0.5) {
                                    [1i32, -1]
                                } else {
                                    [-1, 1]
                                };
                                for &dx in dxs.iter() {
                                    let nx = x as i32 + dx;
                                    if nx >= 0
                                        && nx < WIDTH as i32
                                        && next_grid[y][nx as usize] == Cell::Empty
                                    {
                                        next_grid[y][nx as usize] = Cell::Sand;
                                        moved = true;
                                        break;
                                    }
                                }
                            }
                            if !moved {
                                'save: for dy in -1i32..=1 {
                                    for dx in -1i32..=1 {
                                        let ny = y as i32 + dy;
                                        let nx = x as i32 + dx;
                                        if ny >= 0
                                            && ny < HEIGHT as i32
                                            && nx >= 0
                                            && nx < WIDTH as i32
                                        {
                                            if next_grid[ny as usize][nx as usize] == Cell::Empty {
                                                next_grid[ny as usize][nx as usize] = Cell::Sand;
                                                break 'save;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        self.grid = next_grid;
        if self.state == AppState::Running && self.initial_sand > 0 && self.source_sand() == 0 {
            self.state = AppState::Finished;
        }
    }

    fn source_sand(&self) -> usize {
        let center_y = HEIGHT / 2;
        self.grid
            .iter()
            .enumerate()
            .take(HEIGHT as usize)
            .skip(center_y as usize + 1)
            .flat_map(|(_, row)| row.iter())
            .filter(|&&cell| cell == Cell::Sand)
            .count()
    }

    fn draw_physics(&self, ctx: &mut Context) {
        let mut sand_points = Vec::new();
        let mut wall_points = Vec::new();
        for y in 0..HEIGHT as usize {
            for x in 0..WIDTH as usize {
                match self.grid[y][x] {
                    Cell::Sand => sand_points.push((x as f64, y as f64)),
                    Cell::Wall => wall_points.push((x as f64, y as f64)),
                    _ => {}
                }
            }
        }
        ctx.draw(&Points {
            coords: &wall_points,
            color: Color::Rgb(80, 80, 80),
        });
        ctx.draw(&Points {
            coords: &sand_points,
            color: Color::Yellow,
        });
    }
}

use serde::{Deserialize, Serialize};

// 新しい設定構造体
#[derive(Serialize, Deserialize, Debug, Default)]
struct Config {
    hour: u32,
    minute: u32,
    second: u32,
    wall_friction: f32,
    sand_friction: f32,
    is_24h_format: bool,
}

fn config_path() -> PathBuf {
    let mut p = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    p.push(".sand_timer.toml"); // TOML形式に変更
    p
}

fn save_config(h: u32, m: u32, s: u32, wall_f: f32, sand_f: f32, is_24h: bool) -> Result<()> {
    let config = Config {
        hour: h,
        minute: m,
        second: s,
        wall_friction: wall_f,
        sand_friction: sand_f,
        is_24h_format: is_24h,
    };
    let toml_string = toml::to_string(&config)?;
    fs::write(config_path(), toml_string)?;
    Ok(())
}

fn load_config() -> Config {
    let config_path = config_path();
    if !config_path.exists() {
        return Config::default();
    }
    let content = fs::read_to_string(config_path).unwrap_or_default();
    toml::from_str(&content).unwrap_or_else(|_| Config::default())
}

#[tokio::main]
async fn main() -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let loaded_config = load_config();
    let mut wall_f = loaded_config.wall_friction;
    let mut sand_f = loaded_config.sand_friction;
    let mut app = App::new(
        loaded_config.hour,
        loaded_config.minute,
        loaded_config.second,
        wall_f,
        sand_f,
        loaded_config.is_24h_format,
    );
    let tick_rate = Duration::from_millis(16);

    loop {
        terminal.draw(|f| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(0), Constraint::Length(7)])
                .split(f.area());

            let top_split = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(chunks[0]);

            // Left: Sand Timer
            let ui_width = (WIDTH / 2) as u16 + 2;
            let ui_height = (HEIGHT / 4) as u16 + 2;
            let left_v_margin = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Fill(1),
                    Constraint::Length(ui_height),
                    Constraint::Fill(1),
                ])
                .split(top_split[0]);
            let center_area = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Fill(1),
                    Constraint::Length(ui_width),
                    Constraint::Fill(1),
                ])
                .split(left_v_margin[1])[1];

            let canvas = Canvas::default()
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(" Reliable Sand Timer "),
                )
                .x_bounds([0.0, WIDTH as f64])
                .y_bounds([0.0, HEIGHT as f64])
                .paint(|ctx| app.draw_physics(ctx));
            f.render_widget(canvas, center_area);

            // Right: Digital Clock
            let now = Local::now();
            let clock_str = if app.is_24h {
                now.format("%H:%M:%S").to_string()
            } else {
                now.format("%I:%M:%S %p").to_string()
            };

            // Convert string to big block characters
            fn to_big(s: &str) -> Vec<ratatui::text::Line> {
                let mut lines = vec![String::new(); 5];
                for c in s.chars() {
                    let art = match c {
                        '0' => ["███", "█ █", "█ █", "█ █", "███"],
                        '1' => ["  █", "  █", "  █", "  █", "  █"],
                        '2' => ["███", "  █", "███", "█  ", "███"],
                        '3' => ["███", "  █", "███", "  █", "███"],
                        '4' => ["█ █", "█ █", "███", "  █", "  █"],
                        '5' => ["███", "█  ", "███", "  █", "███"],
                        '6' => ["███", "█  ", "███", "█ █", "███"],
                        '7' => ["███", "  █", "  █", "  █", "  █"],
                        '8' => ["███", "█ █", "███", "█ █", "███"],
                        '9' => ["███", "█ █", "███", "  █", "███"],
                        ':' => ["   ", " ▄ ", "   ", " ▀ ", "   "],
                        'A' => [" ██", "█ █", "███", "█ █", "█ █"],
                        'P' => ["██ ", "█ █", "██ ", "█  ", "█  "],
                        'M' => ["█ █", "███", "█ █", "█ █", "█ █"],
                        ' ' => ["   ", "   ", "   ", "   ", "   "],
                        _   => ["   ", "   ", "   ", "   ", "   "],
                    };
                    for i in 0..5 {
                        lines[i].push_str(art[i]);
                        lines[i].push(' ');
                    }
                }
                lines.into_iter().map(ratatui::text::Line::from).collect()
            }

            let clock_paragraph = Paragraph::new(to_big(&clock_str))
                .alignment(Alignment::Center)
                .style(
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(if app.is_24h { " Digital Clock (24h) " } else { " Digital Clock (12h) " }),
                );

            // Center the clock in the right panel
            let right_v_margin = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Fill(1),
                    Constraint::Length(7), // Increased height for big text
                    Constraint::Fill(1),
                ])
                .split(top_split[1])[1];
            f.render_widget(clock_paragraph, right_v_margin);

            let elapsed_total = app.elapsed.as_secs_f32();
            let eh = (elapsed_total / 3600.0) as u32;
            let em = ((elapsed_total % 3600.0) / 60.0) as u32;
            let es = (elapsed_total % 60.0) as u32;

            let mut timer_spans = vec![ratatui::text::Span::raw(" Set: ")];
            let units = [
                format!("{:02}h", app.h),
                format!("{:02}m", app.m),
                format!("{:02}s", app.s),
            ];
            for (i, unit) in units.iter().enumerate() {
                let style = if app.state == AppState::Setting && app.selected_unit == i {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                timer_spans.push(ratatui::text::Span::styled(unit, style));
                if i < 2 {
                    timer_spans.push(ratatui::text::Span::raw(" : "));
                }
            }
            timer_spans.push(ratatui::text::Span::raw(format!(
                " | Elapsed: {:02}:{:02}:{:02}",
                eh, em, es
            )));

            let fallen_count = app.initial_sand.saturating_sub(app.source_sand());
            let timer_display = vec![
                ratatui::text::Line::from(timer_spans),
                ratatui::text::Line::from(format!(" Particles: {}/{} | Friction(W/S): {:.2}/{:.2} | Rate: {:.1}/s",
                fallen_count, app.initial_sand, wall_f, sand_f, app.grains_per_sec)),
                ratatui::text::Line::from(match app.state {
                    AppState::Setting => " [SETTING] Arrows: Adj, Num: Input, Space: Start, t: Clock 12/24h ",
                    AppState::Running => " [RUNNING] Space: Pause, t: Clock 12/24h ",
                    AppState::Paused  => " [PAUSED] Space: Resume, t: Clock 12/24h ",
                    AppState::Finished => " [FINISHED] r: Reset, t: Clock 12/24h ",
                }).style(Style::default().fg(Color::Yellow)),
            ];

            let stats = Paragraph::new(timer_display)
                .alignment(Alignment::Center)
                .block(Block::default().borders(Borders::ALL).title(" Control Panel "));
            f.render_widget(stats, chunks[1]);
        })?;

        if event::poll(tick_rate)? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') => {
                        save_config(app.h, app.m, app.s, wall_f, sand_f, app.is_24h)?;
                        break;
                    }
                    KeyCode::Char('r') => {
                        app = App::new(app.h, app.m, app.s, wall_f, sand_f, app.is_24h)
                    }
                    KeyCode::Char('t') => {
                        app.is_24h = !app.is_24h;
                        save_config(app.h, app.m, app.s, wall_f, sand_f, app.is_24h)?;
                    }
                    KeyCode::Char(' ') => match app.state {
                        AppState::Setting | AppState::Paused => {
                            if app.initial_sand > 0 {
                                app.state = AppState::Running;
                                app.last_tick = Instant::now();
                            }
                        }
                        AppState::Running => app.state = AppState::Paused,
                        _ => {}
                    },
                    KeyCode::Char(']') => {
                        wall_f = (wall_f + 0.05).min(1.0);
                        app.wall_friction = wall_f;
                        save_config(app.h, app.m, app.s, wall_f, sand_f, app.is_24h)?;
                    }
                    KeyCode::Char('[') => {
                        wall_f = (wall_f - 0.05).max(0.0);
                        app.wall_friction = wall_f;
                        save_config(app.h, app.m, app.s, wall_f, sand_f, app.is_24h)?;
                    }
                    KeyCode::Char('}') => {
                        sand_f = (sand_f + 0.05).min(1.0);
                        app.sand_friction = sand_f;
                        save_config(app.h, app.m, app.s, wall_f, sand_f, app.is_24h)?;
                    }
                    KeyCode::Char('{') => {
                        sand_f = (sand_f - 0.05).max(0.0);
                        app.sand_friction = sand_f;
                        save_config(app.h, app.m, app.s, wall_f, sand_f, app.is_24h)?;
                    }
                    _ if app.state == AppState::Setting => match key.code {
                        KeyCode::Left => app.selected_unit = (app.selected_unit + 2) % 3,
                        KeyCode::Right => app.selected_unit = (app.selected_unit + 1) % 3,
                        KeyCode::Up => {
                            match app.selected_unit {
                                0 => app.h += 1,
                                1 => {
                                    if app.m < 59 {
                                        app.m += 1
                                    } else {
                                        app.h += 1;
                                        app.m = 0
                                    }
                                }
                                2 => {
                                    if app.s < 59 {
                                        app.s += 1
                                    } else {
                                        app.m += 1;
                                        app.s = 0
                                    }
                                }
                                _ => {}
                            }
                            let sel = app.selected_unit;
                            app = App::new(app.h, app.m, app.s, wall_f, sand_f, app.is_24h);
                            app.selected_unit = sel;
                            save_config(app.h, app.m, app.s, wall_f, sand_f, app.is_24h)?;
                        }
                        KeyCode::Down => {
                            match app.selected_unit {
                                0 => {
                                    if app.h > 0 {
                                        app.h -= 1
                                    }
                                }
                                1 => {
                                    if app.m > 0 {
                                        app.m -= 1
                                    } else if app.h > 0 {
                                        app.h -= 1;
                                        app.m = 59
                                    }
                                }
                                2 => {
                                    if app.s > 0 {
                                        app.s -= 1
                                    } else if app.m > 0 {
                                        app.m -= 1;
                                        app.s = 59
                                    }
                                }
                                _ => {}
                            }
                            let sel = app.selected_unit;
                            app = App::new(app.h, app.m, app.s, wall_f, sand_f, app.is_24h);
                            app.selected_unit = sel;
                            save_config(app.h, app.m, app.s, wall_f, sand_f, app.is_24h)?;
                        }
                        KeyCode::Char(c) if c.is_ascii_digit() => {
                            let digit = c.to_digit(10).unwrap();
                            match app.selected_unit {
                                0 => app.h = (app.h * 10 + digit) % 100,
                                1 => {
                                    let v = app.m * 10 + digit;
                                    app.m = if v <= 59 { v } else { digit };
                                }
                                2 => {
                                    let v = app.s * 10 + digit;
                                    app.s = if v <= 59 { v } else { digit };
                                }
                                _ => {}
                            }
                            let sel = app.selected_unit;
                            app = App::new(app.h, app.m, app.s, wall_f, sand_f, app.is_24h);
                            app.selected_unit = sel;
                            save_config(app.h, app.m, app.s, wall_f, sand_f, app.is_24h)?;
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
        }
        app.update();
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}
