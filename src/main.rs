use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use rand::{Rng, RngExt};
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
use std::time::{Duration, Instant};

const WIDTH: i32 = 100;
const HEIGHT: i32 = 160;

#[derive(Clone, Copy, PartialEq)]
enum Cell { Empty, Sand, Wall }

#[derive(PartialEq)]
enum AppState { Setting, Running, Paused, Finished }

struct App {
    grid: Vec<Vec<Cell>>,
    initial_sand: usize,
    dropped_sand: usize,
    flow_accumulator: Duration,
    last_tick: Instant,
    h: u32, m: u32, s: u32,
    selected_unit: usize,
    state: AppState,
    wall_friction: f32,
    sand_friction: f32,
}

impl App {
    fn new(h: u32, m: u32, s: u32, wall_f: f32, sand_f: f32) -> Self {
        let mut grid = vec![vec![Cell::Empty; WIDTH as usize]; HEIGHT as usize];
        let center_x = WIDTH / 2;
        let center_y = HEIGHT / 2;

        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let dx = (x - center_x).abs();
                let dy = (y - center_y).abs();
                if x < 2 || x >= WIDTH - 2 || y < 4 || y >= HEIGHT - 4 {
                    grid[y as usize][x as usize] = Cell::Wall;
                    continue;
                }
                if dy == 0 {
                    if dx > 0 { grid[y as usize][x as usize] = Cell::Wall; }
                } else {
                    if dx as f32 > (dy as f32 / 1.1) {
                        grid[y as usize][x as usize] = Cell::Wall;
                    }
                }
            }
        }

        let total_secs = h * 3600 + m * 60 + s;
        let required_sand = (total_secs as f32 * 10.0).round() as usize;
        let mut count = 0;
        if required_sand > 0 {
            'fill: for y in (center_y + 1)..(HEIGHT - 6) {
                for d in 0..WIDTH/2 {
                    for sign in [-1, 1] {
                        let x = center_x + (d * sign);
                        if x >= 0 && x < WIDTH && grid[y as usize][x as usize] == Cell::Empty {
                            grid[y as usize][x as usize] = Cell::Sand;
                            count += 1;
                            if count >= required_sand { break 'fill; }
                        }
                        if d == 0 { break; }
                    }
                }
            }
        }

        Self {
            grid, initial_sand: count, dropped_sand: 0,
            flow_accumulator: Duration::ZERO, last_tick: Instant::now(),
            h, m, s, selected_unit: 2, state: AppState::Setting,
            wall_friction: wall_f, sand_friction: sand_f,
        }
    }

    fn update(&mut self) {
        if self.state != AppState::Running {
            self.last_tick = Instant::now();
            return;
        }

        let now = Instant::now();
        let delta = now.duration_since(self.last_tick);
        self.last_tick = now;
        self.flow_accumulator += delta;

        let mut rng = rand::rng();
        let mut next_grid = vec![vec![Cell::Empty; WIDTH as usize]; HEIGHT as usize];
        for y in 0..HEIGHT as usize {
            for x in 0..WIDTH as usize {
                if self.grid[y][x] == Cell::Wall { next_grid[y][x] = Cell::Wall; }
            }
        }

        let mut can_pass_gate = self.flow_accumulator >= Duration::from_millis(100);
        let center_x = (WIDTH / 2) as usize;
        let center_y = (HEIGHT / 2) as usize;

        let mut x_range: Vec<usize> = (0..WIDTH as usize).collect();
        if rng.random_bool(0.5) { x_range.reverse(); }

        for y in 0..HEIGHT as usize {
            for &x in &x_range {
                if self.grid[y][x] == Cell::Sand {
                    let mut moved = false;
                    if y > 0 {
                        let is_at_gate = x == center_x && y == center_y;
                        let is_above = y >= center_y;
                        let allowed_to_fall = !is_at_gate || can_pass_gate;

                        if allowed_to_fall && next_grid[y - 1][x] == Cell::Empty {
                            next_grid[y - 1][x] = Cell::Sand;
                            moved = true;
                            if is_at_gate && can_pass_gate {
                                can_pass_gate = false;
                                self.flow_accumulator -= Duration::from_millis(100);
                                self.dropped_sand += 1;
                            }
                        } 
                        else if !is_at_gate {
                            let preferred_dx = if x < center_x { 1 } else if x > center_x { -1 } else { 0 };
                            let dxs = if is_above && rng.random_bool(0.7) {
                                vec![preferred_dx, -preferred_dx]
                            } else {
                                if rng.random_bool(0.5) { vec![1, -1] } else { vec![-1, 1] }
                            };

                            let adj_to_wall = (x > 0 && self.grid[y][x-1] == Cell::Wall) || (x < (WIDTH as usize - 1) && self.grid[y][x+1] == Cell::Wall);
                            let current_friction = if is_above { self.sand_friction * 0.5 } else { self.sand_friction };
                            let friction = if adj_to_wall { self.wall_friction } else { current_friction };

                            if !rng.random_bool(friction as f64) {
                                for &dx in &dxs {
                                    let nx = (x as i32 + dx) as usize;
                                    if nx < WIDTH as usize && next_grid[y - 1][nx] == Cell::Empty {
                                        next_grid[y - 1][nx] = Cell::Sand;
                                        moved = true;
                                        break;
                                    }
                                }
                            }
                        }
                    }

                    if !moved {
                        if next_grid[y][x] == Cell::Empty {
                            next_grid[y][x] = Cell::Sand;
                            moved = true;
                        } else {
                            let is_above = y >= center_y;
                            let vibrate_chance = if is_above { 0.2 } else { 0.05 };
                            if rng.random_bool(vibrate_chance) {
                                let dxs = if rng.random_bool(0.5) { [1, -1] } else { [-1, 1] };
                                for &dx in dxs.iter() {
                                    let nx = (x as i32 + dx) as usize;
                                    if nx < WIDTH as usize && next_grid[y][nx] == Cell::Empty {
                                        next_grid[y][nx] = Cell::Sand;
                                        moved = true;
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    
                    if !moved {
                        'save: for dy in -1..=1 {
                            for dx in -1..=1 {
                                let ny = (y as i32 + dy) as usize;
                                let nx = (x as i32 + dx) as usize;
                                if ny < HEIGHT as usize && nx < WIDTH as usize && next_grid[ny][nx] == Cell::Empty {
                                    next_grid[ny][nx] = Cell::Sand;
                                    moved = true;
                                    break 'save;
                                }
                            }
                        }
                    }
                }
            }
        }
        self.grid = next_grid;
        if self.dropped_sand >= self.initial_sand && self.initial_sand > 0 { self.state = AppState::Finished; }
    }

    fn flip(&mut self) {
        // グリッドを上下反転
        let mut new_grid = vec![vec![Cell::Empty; WIDTH as usize]; HEIGHT as usize];
        for y in 0..HEIGHT as usize {
            for x in 0..WIDTH as usize {
                new_grid[y][x] = self.grid[HEIGHT as usize - 1 - y][x];
            }
        }
        self.grid = new_grid;
        // 上半分（これから落ちる側）の砂をinitial_sandとして再計算
        let center_y = HEIGHT as usize / 2;
        self.initial_sand = self.grid[center_y..].iter().flatten().filter(|&&c| c == Cell::Sand).count();
        self.dropped_sand = 0;
        self.flow_accumulator = Duration::ZERO;
        self.state = AppState::Running;
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
        ctx.draw(&Points { coords: &wall_points, color: Color::Rgb(80, 80, 80) });
        ctx.draw(&Points { coords: &sand_points, color: Color::Yellow });
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut wall_f = 0.1;
    let mut sand_f = 0.02;
    let mut app = App::new(0, 0, 30, wall_f, sand_f);
    let tick_rate = Duration::from_millis(16);

    loop {
        terminal.draw(|f| {
            let ui_width = (WIDTH / 2) as u16 + 2;
            let ui_height = (HEIGHT / 4) as u16 + 2;
            let chunks = Layout::default().direction(Direction::Vertical).constraints([Constraint::Min(0), Constraint::Length(7)]).split(f.area());
            let vertical_margin = Layout::default().direction(Direction::Vertical).constraints([Constraint::Fill(1), Constraint::Length(ui_height), Constraint::Fill(1)]).split(chunks[0]);
            let center_area = Layout::default().direction(Direction::Horizontal).constraints([Constraint::Fill(1), Constraint::Length(ui_width), Constraint::Fill(1)]).split(vertical_margin[1])[1];

            let canvas = Canvas::default().block(Block::default().borders(Borders::ALL).title(" Reliable Sand Timer ")).x_bounds([0.0, WIDTH as f64]).y_bounds([0.0, HEIGHT as f64]).paint(|ctx| app.draw_physics(ctx));
            f.render_widget(canvas, center_area);

            let elapsed_total = app.dropped_sand as f32 * 0.1;
            let eh = (elapsed_total / 3600.0) as u32;
            let em = ((elapsed_total % 3600.0) / 60.0) as u32;
            let es = (elapsed_total % 60.0) as u32;

            let mut timer_spans = vec![ratatui::text::Span::raw(" Set: ")];
            let units = [format!("{:02}h", app.h), format!("{:02}m", app.m), format!("{:02}s", app.s)];
            for (i, unit) in units.iter().enumerate() {
                let style = if app.state == AppState::Setting && app.selected_unit == i {
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
                } else { Style::default() };
                timer_spans.push(ratatui::text::Span::styled(unit, style));
                if i < 2 { timer_spans.push(ratatui::text::Span::raw(" : ")); }
            }
            timer_spans.push(ratatui::text::Span::raw(format!(" | Elapsed: {:02}:{:02}:{:02}", eh, em, es)));

            let timer_display = vec![
                ratatui::text::Line::from(timer_spans),
                ratatui::text::Line::from(format!(" Particles: {}/{} | Friction(W/S): {:.2}/{:.2}", app.dropped_sand, app.initial_sand, wall_f, sand_f)),
                ratatui::text::Line::from(match app.state {
                    AppState::Setting => " [SETTING] Arrows: Adj, Num: Input, Space: Start ",
                    AppState::Running => " [RUNNING] Space: Pause  f: Flip ",
                    AppState::Paused => " [PAUSED] Space: Resume  f: Flip ",
                    AppState::Finished => " [FINISHED] f: Flip  r: Reset ",
                }).style(Style::default().fg(Color::Yellow)),
            ];

            let stats = Paragraph::new(timer_display).alignment(Alignment::Center).block(Block::default().borders(Borders::ALL).title(" Control Panel "));
            f.render_widget(stats, chunks[1]);
        })?;

        if event::poll(tick_rate)? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') => break,
                    KeyCode::Char('r') => app = App::new(app.h, app.m, app.s, wall_f, sand_f),
                    KeyCode::Char('f') if app.state != AppState::Setting => app.flip(),
                    KeyCode::Char(' ') => match app.state {
                        AppState::Setting | AppState::Paused => if app.initial_sand > 0 { app.state = AppState::Running },
                        AppState::Running => app.state = AppState::Paused,
                        _ => {}
                    },
                    KeyCode::Char(']') => { wall_f = (wall_f + 0.05).min(1.0); app.wall_friction = wall_f; },
                    KeyCode::Char('[') => { wall_f = (wall_f - 0.05).max(0.0); app.wall_friction = wall_f; },
                    KeyCode::Char('}') => { sand_f = (sand_f + 0.05).min(1.0); app.sand_friction = sand_f; },
                    KeyCode::Char('{') => { sand_f = (sand_f - 0.05).max(0.0); app.sand_friction = sand_f; },
                    _ if app.state == AppState::Setting => {
                        match key.code {
                            KeyCode::Left => app.selected_unit = (app.selected_unit + 2) % 3,
                            KeyCode::Right => app.selected_unit = (app.selected_unit + 1) % 3,
                            KeyCode::Up => {
                                match app.selected_unit {
                                    0 => app.h += 1,
                                    1 => if app.m < 59 { app.m += 1 } else { app.h += 1; app.m = 0 },
                                    2 => if app.s < 59 { app.s += 1 } else { app.m += 1; app.s = 0 },
                                    _ => {}
                                }
                                let sel = app.selected_unit;
                                app = App::new(app.h, app.m, app.s, wall_f, sand_f);
                                app.selected_unit = sel;
                            }
                            KeyCode::Down => {
                                match app.selected_unit {
                                    0 => if app.h > 0 { app.h -= 1 },
                                    1 => if app.m > 0 { app.m -= 1 } else if app.h > 0 { app.h -= 1; app.m = 59 },
                                    2 => if app.s > 0 { app.s -= 1 } else if app.m > 0 { app.m -= 1; app.s = 59 },
                                    _ => {}
                                }
                                let sel = app.selected_unit;
                                app = App::new(app.h, app.m, app.s, wall_f, sand_f);
                                app.selected_unit = sel;
                            }
                            KeyCode::Char(c) if c.is_ascii_digit() => {
                                let digit = c.to_digit(10).unwrap();
                                match app.selected_unit {
                                    0 => app.h = (app.h * 10 + digit) % 100,
                                    1 => { let v = app.m * 10 + digit; app.m = if v <= 59 { v } else { digit }; },
                                    2 => { let v = app.s * 10 + digit; app.s = if v <= 59 { v } else { digit }; },
                                    _ => {}
                                }
                                let sel = app.selected_unit;
                                app = App::new(app.h, app.m, app.s, wall_f, sand_f);
                                app.selected_unit = sel;
                            }
                            _ => {}
                        }
                    }
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
