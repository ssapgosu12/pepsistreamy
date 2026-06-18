//! 설정 TUI (ratatui + crossterm). 방향키 + 엔터로 토큰/소스/DSP/모니터를 설정한다.
//! 토큰은 입력 즉시 DPAPI 암호화되어 setting.ini 에 저장된다.

use anyhow::Result;
use ratatui::DefaultTerminal;
use ratatui::Frame;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Gauge, List, ListItem, ListState, Paragraph, Wrap};

use crate::dsp::DspParams;
use crate::settings::{Settings, SourceSel};
use crate::setup;

const DSP_LABELS: [&str; 6] = [
    "HighFreq (HPF 컷오프)",
    "HighRes  (HPF 레조넌스/Q)",
    "LowFreq  (LPF 컷오프)",
    "LowRes   (LPF 레조넌스/Q)",
    "RevRoomSize (리버브 룸)",
    "RevMix   (리버브 섞임)",
];

fn dsp_get(p: &DspParams, i: usize) -> u8 {
    [
        p.high_freq,
        p.high_res,
        p.low_freq,
        p.low_res,
        p.room,
        p.mix,
    ][i]
}
fn dsp_set(p: &mut DspParams, i: usize, v: u8) {
    match i {
        0 => p.high_freq = v,
        1 => p.high_res = v,
        2 => p.low_freq = v,
        3 => p.low_res = v,
        4 => p.room = v,
        _ => p.mix = v,
    }
}

#[derive(PartialEq)]
enum Screen {
    Menu,
    Token,
    SourceKind,
    DevicePick,
    ProcessPick,
    Dsp,
    ProfileSave,
    ProfileList,
    Monitor,
    MonitorDevicePick,
}

struct App {
    s: Settings,
    screen: Screen,
    sel: usize,          // 현재 화면의 리스트/항목 선택 인덱스
    items: Vec<String>,  // 표시용 리스트(장치/프로세스/프로필)
    values: Vec<String>, // 항목별 실제 값(프로세스는 이름만 저장)
    input: String,       // 토큰/프로필명 입력
    msg: String,
    proc_all: bool, // 프로세스 픽커: 전체(true) vs 소리나는 앱만(false)
    start_bot: bool,
    quit: bool,
}

/// 설정 화면(블로킹). true = "저장하고 봇 실행" 선택됨.
pub fn config() -> Result<bool> {
    let mut app = App {
        s: Settings::load(),
        screen: Screen::Menu,
        sel: 0,
        items: Vec::new(),
        values: Vec::new(),
        input: String::new(),
        msg: String::new(),
        proc_all: false,
        start_bot: false,
        quit: false,
    };
    let mut terminal = ratatui::init();
    let res = app.event_loop(&mut terminal);
    ratatui::restore();
    res?;
    app.s.save().ok();
    Ok(app.start_bot)
}

const MENU: [&str; 7] = [
    "1. 디스코드 토큰 설정",
    "2. 캡처 소스 (기본/장치/프로세스/레거시)",
    "3. DSP 필터 (HP/LP/리버브 + 프로필)",
    "4. 로컬 모니터 (송출자도 필터본 듣기)",
    "5. 저장",
    "6. 저장하고 봇 실행",
    "7. 종료",
];

impl App {
    fn event_loop(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        loop {
            terminal.draw(|f| self.render(f))?;
            if let Event::Key(k) = event::read()? {
                if k.kind == KeyEventKind::Press {
                    self.on_key(k.code);
                }
            }
            if self.quit {
                break;
            }
        }
        Ok(())
    }

    fn back_to_menu(&mut self) {
        self.screen = Screen::Menu;
        self.sel = 0;
        self.input.clear();
    }

    fn on_key(&mut self, code: KeyCode) {
        // 텍스트 입력 화면은 별도 처리
        match self.screen {
            Screen::Token | Screen::ProfileSave => return self.on_key_input(code),
            _ => {}
        }
        match code {
            KeyCode::Esc => {
                if self.screen == Screen::Menu {
                    self.quit = true;
                } else {
                    self.back_to_menu();
                }
            }
            KeyCode::Char('q') if self.screen == Screen::Menu => self.quit = true,
            KeyCode::Up => self.move_sel(-1),
            KeyCode::Down => self.move_sel(1),
            KeyCode::Left => self.adjust(-1),
            KeyCode::Right => self.adjust(1),
            KeyCode::Char('-') => self.adjust(-1),
            KeyCode::Char('+') | KeyCode::Char('=') => self.adjust(1),
            KeyCode::Enter => self.enter(),
            KeyCode::Char(c) => self.on_extra_key(c),
            _ => {}
        }
    }

    fn list_len(&self) -> usize {
        match self.screen {
            Screen::Menu => MENU.len(),
            Screen::SourceKind => 4,
            Screen::DevicePick
            | Screen::ProcessPick
            | Screen::ProfileList
            | Screen::MonitorDevicePick => self.items.len(),
            Screen::Dsp => 7,     // enabled + 6 params
            Screen::Monitor => 2, // enabled + device
            _ => 0,
        }
    }

    fn move_sel(&mut self, d: i32) {
        let n = self.list_len();
        if n == 0 {
            return;
        }
        let cur = self.sel as i32;
        self.sel = ((cur + d).rem_euclid(n as i32)) as usize;
    }

    fn adjust(&mut self, d: i32) {
        if self.screen == Screen::Dsp && self.sel >= 1 {
            let i = self.sel - 1;
            let v = dsp_get(&self.s.dsp, i) as i32 + d;
            dsp_set(&mut self.s.dsp, i, v.clamp(0, 100) as u8);
            self.s.dsp_enabled = true;
        }
    }

    fn on_extra_key(&mut self, c: char) {
        match (&self.screen, c) {
            (Screen::Dsp, 's') | (Screen::Dsp, 'S') => {
                self.screen = Screen::ProfileSave;
                self.input.clear();
            }
            (Screen::Dsp, 'l') | (Screen::Dsp, 'L') => {
                self.items = self.s.profiles.keys().cloned().collect();
                self.values = self.items.clone();
                self.sel = 0;
                self.screen = Screen::ProfileList;
                if self.items.is_empty() {
                    self.msg = "저장된 프로필이 없습니다.".into();
                    self.back_to_menu_keep_msg(Screen::Dsp);
                }
            }
            (Screen::Dsp, 't') | (Screen::Dsp, 'T') => {
                self.s.dsp_enabled = !self.s.dsp_enabled;
            }
            (Screen::ProcessPick, 'a') | (Screen::ProcessPick, 'A') => {
                self.proc_all = !self.proc_all;
                self.load_process_list();
            }
            (Screen::SourceKind, 'o') | (Screen::Monitor, 'o') => {
                setup::open_url(setup::VBCABLE_URL);
                self.msg = "VB-CABLE 다운로드 페이지를 열었습니다(VB-Audio 도네이션웨어).".into();
            }
            _ => {}
        }
    }

    fn back_to_menu_keep_msg(&mut self, screen: Screen) {
        self.screen = screen;
    }

    fn enter(&mut self) {
        match self.screen {
            Screen::Menu => self.menu_enter(),
            Screen::SourceKind => self.source_kind_enter(),
            Screen::DevicePick => {
                if let Some(name) = self.values.get(self.sel).cloned() {
                    self.s.source = SourceSel::Device(name);
                    self.msg = "출력장치 캡처로 설정.".into();
                    self.back_to_menu();
                }
            }
            Screen::ProcessPick => {
                if let Some(name) = self.values.get(self.sel).cloned() {
                    self.s.source = SourceSel::Process(name.clone());
                    self.msg = format!("프로세스 '{name}' 캡처로 설정(실행 시 루트 자동 선택).");
                    self.back_to_menu();
                }
            }
            Screen::ProfileList => {
                if let Some(name) = self.values.get(self.sel).cloned() {
                    if let Some(p) = self.s.profiles.get(&name).copied() {
                        self.s.dsp = p;
                        self.s.dsp_enabled = true;
                        self.msg = format!("프로필 '{name}' 적용.");
                    }
                    self.screen = Screen::Dsp;
                    self.sel = 0;
                }
            }
            Screen::Dsp => {
                if self.sel == 0 {
                    self.s.dsp_enabled = !self.s.dsp_enabled;
                }
            }
            Screen::Monitor => {
                if self.sel == 0 {
                    self.s.monitor = !self.s.monitor;
                } else {
                    self.items = std::iter::once("(기본 출력장치)".to_string())
                        .chain(crate::capture::list_render_devices().unwrap_or_default())
                        .collect();
                    self.values = self.items.clone();
                    self.sel = 0;
                    self.screen = Screen::MonitorDevicePick;
                }
            }
            Screen::MonitorDevicePick => {
                if let Some(name) = self.values.get(self.sel).cloned() {
                    self.s.monitor_device = if self.sel == 0 { None } else { Some(name) };
                    self.screen = Screen::Monitor;
                    self.sel = 0;
                }
            }
            _ => {}
        }
    }

    fn menu_enter(&mut self) {
        self.msg.clear();
        match self.sel {
            0 => {
                self.screen = Screen::Token;
                self.input.clear();
            }
            1 => {
                self.screen = Screen::SourceKind;
                self.sel = 0;
            }
            2 => {
                self.screen = Screen::Dsp;
                self.sel = 0;
            }
            3 => {
                self.screen = Screen::Monitor;
                self.sel = 0;
            }
            4 => {
                self.msg = match self.s.save() {
                    Ok(_) => "setting.ini 저장됨.".into(),
                    Err(e) => format!("저장 실패: {e}"),
                };
            }
            5 => {
                if self.s.token().is_none() && !self.s.has_token() {
                    self.msg = "토큰을 먼저 설정하세요.".into();
                } else {
                    self.start_bot = true;
                    self.quit = true;
                }
            }
            _ => self.quit = true,
        }
    }

    fn load_process_list(&mut self) {
        // 기본: 소리 나는 앱만(오디오 세션 있는). 없으면 전체. 'a' 로 전체 토글.
        let named = if self.proc_all {
            crate::process::list_named()
        } else {
            let audio = crate::process::list_audio();
            if audio.is_empty() {
                crate::process::list_named()
            } else {
                audio
            }
        };
        self.items = named
            .iter()
            .map(|(name, n)| {
                if *n > 1 {
                    format!("{name}  ({n})")
                } else {
                    name.clone()
                }
            })
            .collect();
        self.values = named.into_iter().map(|(name, _)| name).collect();
        self.sel = 0;
    }

    fn source_kind_enter(&mut self) {
        match self.sel {
            0 => {
                self.s.source = SourceSel::Default;
                self.msg = "기본 스피커(시스템 전체 믹스)로 설정.".into();
                self.back_to_menu();
            }
            1 => {
                self.items = crate::capture::list_render_devices().unwrap_or_default();
                self.values = self.items.clone();
                self.sel = 0;
                self.screen = Screen::DevicePick;
            }
            2 => {
                self.proc_all = false;
                self.load_process_list();
                self.screen = Screen::ProcessPick;
            }
            _ => {
                // 레거시(VB-CABLE)
                let cable = crate::capture::list_render_devices()
                    .unwrap_or_default()
                    .into_iter()
                    .find(|n| n.to_uppercase().contains("CABLE"));
                match cable {
                    Some(name) => {
                        self.s.source = SourceSel::Legacy(name);
                        self.msg =
                            "레거시(VB-CABLE) 캡처로 설정. 그 앱 출력을 CABLE 로 보내세요.".into();
                        self.back_to_menu();
                    }
                    None => {
                        self.msg =
                            "VB-CABLE 미설치. 'o' 키로 공식 다운로드 페이지를 여세요.".into();
                    }
                }
            }
        }
    }

    fn on_key_input(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => self.back_to_menu(),
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Char(c) => self.input.push(c),
            KeyCode::Enter => {
                let val = self.input.trim().to_string();
                match self.screen {
                    Screen::Token => {
                        if val.is_empty() {
                            self.msg = "토큰이 비어 있습니다.".into();
                        } else {
                            match self.s.set_token(&val) {
                                Ok(_) => {
                                    self.s.save().ok();
                                    // 토큰에서 client_id 디코드 → 초대 URL 자동 오픈 제안
                                    self.msg = match setup::client_id_from_token(&val) {
                                        Some(id) => {
                                            setup::open_url(&setup::invite_url(&id));
                                            "토큰 암호화 저장됨. 초대 페이지를 열었습니다.".into()
                                        }
                                        None => "토큰 암호화 저장됨.".into(),
                                    };
                                }
                                Err(e) => self.msg = format!("암호화 실패: {e}"),
                            }
                        }
                        self.back_to_menu();
                    }
                    Screen::ProfileSave => {
                        if !val.is_empty() {
                            self.s.profiles.insert(val.clone(), self.s.dsp);
                            self.msg = format!("프로필 '{val}' 저장.");
                        }
                        self.screen = Screen::Dsp;
                        self.input.clear();
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    // ---- 렌더 ----

    fn render(&mut self, f: &mut Frame) {
        let area = f.area();
        let chunks = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(3),
        ])
        .split(area);

        let title =
            Paragraph::new("PepsiStreamy 설정 — ↑↓ 이동 · ←→ 값조정 · Enter 선택 · Esc 뒤로/종료")
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("🥤 PepsiStreamy"),
                );
        f.render_widget(title, chunks[0]);

        match self.screen {
            Screen::Menu => self.render_menu(f, chunks[1]),
            Screen::Token => self.render_token(f, chunks[1]),
            Screen::SourceKind => self.render_list(
                f,
                chunks[1],
                "캡처 소스",
                &[
                    "기본 스피커 (시스템 전체 믹스)".into(),
                    "특정 출력장치 선택".into(),
                    "특정 프로세스(앱) 선택".into(),
                    "레거시: VB-CABLE  ('o'=다운로드)".into(),
                ],
            ),
            Screen::DevicePick => {
                let items = self.items.clone();
                self.render_list(f, chunks[1], "출력장치 선택", &items)
            }
            Screen::ProcessPick => {
                let items = self.items.clone();
                let title = if self.proc_all {
                    "프로세스 선택 — 전체  (a: 소리나는 앱만)"
                } else {
                    "프로세스 선택 — 🔊 소리나는 앱만  (a: 전체보기)"
                };
                self.render_list(f, chunks[1], title, &items)
            }
            Screen::ProfileList => {
                let items = self.items.clone();
                self.render_list(f, chunks[1], "프로필 불러오기", &items)
            }
            Screen::Dsp => self.render_dsp(f, chunks[1]),
            Screen::Monitor => self.render_monitor(f, chunks[1]),
            Screen::MonitorDevicePick => {
                let items = self.items.clone();
                self.render_list(f, chunks[1], "모니터 출력장치", &items)
            }
            Screen::ProfileSave => self.render_input(f, chunks[1], "프로필 이름 입력 후 Enter"),
        }

        let help = Paragraph::new(self.status_line())
            .block(Block::default().borders(Borders::ALL).title("상태"))
            .wrap(Wrap { trim: true });
        f.render_widget(help, chunks[2]);
    }

    fn status_line(&self) -> String {
        let token = if self.s.has_token() {
            "설정됨(암호화)"
        } else {
            "없음"
        };
        let src = match &self.s.source {
            SourceSel::Default => "기본 스피커".to_string(),
            SourceSel::Device(n) => format!("장치:{n}"),
            SourceSel::Process(p) => format!("프로세스:{p}"),
            SourceSel::Legacy(n) => format!("레거시:{n}"),
        };
        let mon = if self.s.monitor {
            format!(
                "모니터:켜짐({})",
                self.s.monitor_device.clone().unwrap_or("기본".into())
            )
        } else {
            "모니터:꺼짐".to_string()
        };
        let dsp = if self.s.dsp_enabled {
            "DSP:켜짐"
        } else {
            "DSP:꺼짐"
        };
        if self.msg.is_empty() {
            format!("토큰:{token} | 소스:{src} | {dsp} | {mon}")
        } else {
            format!("{}  ·  토큰:{token} | 소스:{src} | {dsp} | {mon}", self.msg)
        }
    }

    fn render_menu(&mut self, f: &mut Frame, area: Rect) {
        let items: Vec<ListItem> = MENU.iter().map(|m| ListItem::new(*m)).collect();
        self.render_list_widget(f, area, "메뉴", items);
    }

    fn render_list(&mut self, f: &mut Frame, area: Rect, title: &str, list: &[String]) {
        let items: Vec<ListItem> = if list.is_empty() {
            vec![ListItem::new("(항목 없음 — Esc 로 뒤로)")]
        } else {
            list.iter().map(|m| ListItem::new(m.clone())).collect()
        };
        self.render_list_widget(f, area, title, items);
    }

    fn render_list_widget(&mut self, f: &mut Frame, area: Rect, title: &str, items: Vec<ListItem>) {
        let mut state = ListState::default();
        state.select(Some(self.sel.min(items.len().saturating_sub(1))));
        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(title.to_string()),
            )
            .highlight_style(
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("➤ ");
        f.render_stateful_widget(list, area, &mut state);
    }

    fn render_token(&mut self, f: &mut Frame, area: Rect) {
        let shown = "*".repeat(self.input.chars().count().min(48));
        let body = vec![
            Line::from(
                "봇 토큰을 붙여넣고 Enter (Esc=취소). 입력값은 DPAPI로 암호화되어 저장됩니다.",
            ),
            Line::from(""),
            Line::from(format!("토큰: {shown}")),
            Line::from(""),
            Line::from(format!(
                "개발자 포털: {}  (Bot > Reset Token)",
                setup::DEV_PORTAL
            )),
        ];
        let p = Paragraph::new(body)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("디스코드 토큰"),
            )
            .wrap(Wrap { trim: true });
        f.render_widget(p, area);
    }

    fn render_input(&mut self, f: &mut Frame, area: Rect, title: &str) {
        let p = Paragraph::new(format!("{}\n\n> {}", title, self.input))
            .block(Block::default().borders(Borders::ALL).title("입력"))
            .wrap(Wrap { trim: true });
        f.render_widget(p, area);
    }

    fn render_dsp(&mut self, f: &mut Frame, area: Rect) {
        let p = &self.s.dsp;
        let rows = Layout::vertical([
            Constraint::Length(2), // enabled + 매핑 정보
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1), // 도움말
        ])
        .split(area);

        let en = if self.s.dsp_enabled {
            "[켜짐]"
        } else {
            "[꺼짐]"
        };
        let info = Paragraph::new(format!(
            "{en} (Enter=토글, t)   HP {:.0}Hz Q{:.1} · LP {:.0}Hz Q{:.1} · room {:.2} · mix {:.2}",
            p.hp_hz(),
            p.hp_q(),
            p.lp_hz(),
            p.lp_q(),
            p.room01(),
            p.mix01(),
        ))
        .style(if self.sel == 0 {
            Style::default().fg(Color::Black).bg(Color::Cyan)
        } else {
            Style::default()
        });
        f.render_widget(info, rows[0]);

        for i in 0..6 {
            let v = dsp_get(p, i);
            let selected = self.sel == i + 1;
            let style = if selected {
                Style::default().fg(Color::Black).bg(Color::Cyan)
            } else {
                Style::default().fg(Color::Cyan)
            };
            let g = Gauge::default()
                .gauge_style(style)
                .ratio(v as f64 / 100.0)
                .label(format!(
                    "{}{}: {v}",
                    if selected { "➤ " } else { "  " },
                    DSP_LABELS[i]
                ));
            f.render_widget(g, rows[i + 1]);
        }

        let help = Paragraph::new(
            "←→/+- 값조정 · t 켜기끄기 · s 프로필저장 · l 프로필불러오기 · Esc 뒤로",
        )
        .wrap(Wrap { trim: true });
        f.render_widget(help, rows[7]);
    }

    fn render_monitor(&mut self, f: &mut Frame, area: Rect) {
        let on = if self.s.monitor { "켜짐" } else { "꺼짐" };
        let dev = self
            .s
            .monitor_device
            .clone()
            .unwrap_or("기본 출력장치".into());
        let items = vec![
            ListItem::new(format!("모니터: {on}   (Enter=토글)")),
            ListItem::new(format!("출력장치: {dev}   (Enter=선택)")),
        ];
        let mut state = ListState::default();
        state.select(Some(self.sel.min(1)));
        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title(
                "로컬 모니터 — 송출자도 필터된 소리 듣기 (프로세스캡처+앱 볼륨믹서 뮤트 권장)",
            ))
            .highlight_style(Style::default().fg(Color::Black).bg(Color::Cyan))
            .highlight_symbol("➤ ");
        f.render_stateful_widget(list, area, &mut state);
    }
}

// ===== 봇 실행 화면 (라이브 볼륨바 + Esc 로 설정 복귀) =====

pub enum Outcome {
    Config, // 설정 메뉴로 복귀
    Quit,   // 완전 종료
}

/// 봇을 백그라운드로 실행하고 상태/볼륨 화면을 띄운다. Esc/q=설정, Ctrl+C=종료.
pub async fn run_bot_screen() -> Result<Outcome> {
    let (token, guild_id) = match crate::bot::token_and_guild() {
        Ok(v) => v,
        Err(_) => return Ok(Outcome::Config),
    };
    crate::bot::set_quiet(true);
    let client = crate::bot::build_client(&token, guild_id).await?;
    let shard = client.shard_manager.clone();
    let bot_task = tokio::spawn(async move {
        let mut client = client;
        let _ = client.start().await;
    });

    let outcome = tokio::task::spawn_blocking(running_loop)
        .await
        .unwrap_or(Outcome::Quit);

    shard.shutdown_all().await;
    bot_task.abort();
    crate::bot::stop_capture();
    crate::bot::set_quiet(false);
    Ok(outcome)
}

fn running_loop() -> Outcome {
    let mut terminal = ratatui::init();
    let out = loop {
        let _ = terminal.draw(draw_running);
        if event::poll(std::time::Duration::from_millis(100)).unwrap_or(false) {
            if let Ok(Event::Key(k)) = event::read() {
                if k.kind != KeyEventKind::Press {
                    continue;
                }
                match k.code {
                    KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                        break Outcome::Quit;
                    }
                    KeyCode::Esc | KeyCode::Char('q') => break Outcome::Config,
                    _ => {}
                }
            }
        }
    };
    ratatui::restore();
    out
}

fn draw_running(f: &mut Frame) {
    let area = f.area();
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(4),
        Constraint::Length(3),
        Constraint::Min(1),
    ])
    .split(area);

    let title =
        Paragraph::new("🥤 PepsiStreamy — 방송 중").block(Block::default().borders(Borders::ALL));
    f.render_widget(title, chunks[0]);

    let connected = crate::bot::is_connected();
    let info = Paragraph::new(vec![
        Line::from(format!(
            "연결: {}   청취자: {}명",
            if connected {
                "음성채널 연결됨"
            } else {
                "대기중 (디스코드에서 /join)"
            },
            crate::bot::listeners()
        )),
        Line::from(format!("상태: {}", crate::bot::status())),
    ])
    .block(Block::default().borders(Borders::ALL).title("상태"))
    .wrap(Wrap { trim: true });
    f.render_widget(info, chunks[1]);

    let level = crate::capture::current_level();
    let db = if level < 1e-4 {
        -90.0
    } else {
        20.0 * level.log10()
    };
    let gauge = Gauge::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("입력 레벨 (필터 후)"),
        )
        .gauge_style(Style::default().fg(Color::Green))
        .ratio(level.sqrt().clamp(0.0, 1.0) as f64)
        .label(format!("{db:.0} dBFS"));
    f.render_widget(gauge, chunks[2]);

    let help = Paragraph::new(
        "Esc/q → 설정으로 · Ctrl+C → 완전 종료   |   디스코드: /join  /reload  /leave  /status",
    )
    .block(Block::default().borders(Borders::ALL))
    .wrap(Wrap { trim: true });
    f.render_widget(help, chunks[3]);
}
