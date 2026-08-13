# 05 — Command Palette Improvement

## Goal

Fix command execution from the command palette, reduce its width, and add icons to commands to make it look better.

## Current State

Almost none of the commands work when triggered from the command palette, suggesting it's a stub implementation. The palette is also too wide and lacks visual polish.

## Required Changes

### 1. Analyze Current Implementation

Review how commands are handled when pressing Enter on the currently selected item:

```rust
// In gtm/src/overlay.rs
impl CommandPalette {
    pub fn execute_selected(&self, app: &mut App) -> Result<(), AppError> {
        let command = &self.commands[self.selected];
        
        // This is likely where the stub is
        match command.action {
            CommandAction::PlayPause => {
                app.client.send(DaemonReq::PlayPause)?;
            }
            // ... other commands
        }
        
        Ok(())
    }
}
```

### 2. Fix Command Execution

Ensure all commands are properly wired up:

```rust
#[derive(Debug, Clone)]
pub enum CommandAction {
    // Playback
    PlayPause,
    Next,
    Prev,
    Stop,
    SeekForward,
    SeekBackward,
    VolumeUp,
    VolumeDown,
    ToggleMute,
    ToggleShuffle,
    CycleRepeat,
    
    // Navigation
    NextTab,
    PrevTab,
    SwitchToNowPlaying,
    SwitchToLibrary,
    SwitchToSettings,
    
    // Overlays
    OpenQueue,
    OpenYTSearch,
    OpenSearchLibrary,
    OpenEqualizer,
    OpenAbout,
    OpenSleepTimer,
    OpenThemePicker,
    OpenLyrics,
    
    // Visualizer
    ToggleVisualizer,
    
    // Settings
    ToggleHelpRow,
    ChangeProgressStyle,
    
    // System
    Quit,
}

impl App {
    fn execute_command(&mut self, action: CommandAction) -> Result<(), AppError> {
        match action {
            CommandAction::PlayPause => {
                self.client.send(DaemonReq::PlayPause)?;
            }
            CommandAction::Next => {
                self.client.send(DaemonReq::Next)?;
            }
            CommandAction::Prev => {
                self.client.send(DaemonReq::Prev)?;
            }
            CommandAction::Stop => {
                self.client.send(DaemonReq::Stop)?;
            }
            CommandAction::SeekForward => {
                let new_pos = (self.state.time_pos + 5.0).min(self.state.duration);
                self.client.send(DaemonReq::Seek(new_pos))?;
            }
            CommandAction::SeekBackward => {
                let new_pos = (self.state.time_pos - 5.0).max(0.0);
                self.client.send(DaemonReq::Seek(new_pos))?;
            }
            CommandAction::VolumeUp => {
                let new_vol = (self.state.volume + 5).min(100);
                self.client.send(DaemonReq::SetVolume(new_vol))?;
            }
            CommandAction::VolumeDown => {
                let new_vol = (self.state.volume - 5).max(0);
                self.client.send(DaemonReq::SetVolume(new_vol))?;
            }
            CommandAction::ToggleMute => {
                self.client.send(DaemonReq::ToggleMute)?;
            }
            CommandAction::ToggleShuffle => {
                self.client.send(DaemonReq::ToggleShuffle)?;
            }
            CommandAction::CycleRepeat => {
                let next_mode = self.state.repeat.next();
                self.client.send(DaemonReq::CycleRepeat(next_mode))?;
            }
            CommandAction::NextTab => {
                self.state.current_tab = self.state.current_tab.next();
            }
            CommandAction::PrevTab => {
                self.state.current_tab = self.state.current_tab.prev();
            }
            CommandAction::SwitchToNowPlaying => {
                self.state.current_tab = Tab::NowPlaying;
            }
            CommandAction::SwitchToLibrary => {
                self.state.current_tab = Tab::Library;
            }
            CommandAction::SwitchToSettings => {
                self.state.current_tab = Tab::Settings;
            }
            CommandAction::OpenQueue => {
                self.overlay = Some(Overlay::Queue);
            }
            CommandAction::OpenYTSearch => {
                self.overlay = Some(Overlay::YTSearch);
            }
            CommandAction::OpenSearchLibrary => {
                self.overlay = Some(Overlay::SearchLibrary);
            }
            CommandAction::OpenEqualizer => {
                self.overlay = Some(Overlay::Equalizer);
            }
            CommandAction::OpenAbout => {
                self.overlay = Some(Overlay::About);
            }
            CommandAction::OpenSleepTimer => {
                self.overlay = Some(Overlay::SleepTimer);
            }
            CommandAction::OpenThemePicker => {
                self.overlay = Some(Overlay::ThemePicker);
            }
            CommandAction::OpenLyrics => {
                self.overlay = Some(Overlay::Lyrics);
            }
            CommandAction::ToggleVisualizer => {
                self.visualizer.toggle();
            }
            CommandAction::ToggleHelpRow => {
                self.show_help_row = !self.show_help_row;
            }
            CommandAction::ChangeProgressStyle => {
                // Open progress style selector
                self.overlay = Some(Overlay::ProgressStylePicker);
            }
            CommandAction::Quit => {
                self.should_quit = true;
            }
        }
        
        // Close command palette after execution
        self.overlay = None;
        
        Ok(())
    }
}
```

### 3. Add Icons to Commands

Use nerd font icons with emoji fallback:

```rust
#[derive(Debug, Clone)]
pub struct Command {
    pub name: String,
    pub icon: String,
    pub keybinding: Option<String>,
    pub action: CommandAction,
    pub category: CommandCategory,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandCategory {
    Playback,
    Navigation,
    Overlays,
    Visualizer,
    Settings,
    System,
}

impl Command {
    pub fn new(name: &str, icon: &str, action: CommandAction) -> Self {
        Self {
            name: name.to_string(),
            icon: icon.to_string(),
            keybinding: None,
            action,
            category: CommandCategory::Playback,
        }
    }
    
    pub fn with_keybinding(mut self, key: &str) -> Self {
        self.keybinding = Some(key.to_string());
        self
    }
    
    pub fn with_category(mut self, category: CommandCategory) -> Self {
        self.category = category;
        self
    }
}
```

#### Icon Set

```rust
pub fn get_commands() -> Vec<Command> {
    vec![
        // Playback
        Command::new("Play/Pause", "▶", CommandAction::PlayPause)
            .with_keybinding("Space")
            .with_category(CommandCategory::Playback),
        Command::new("Next Track", "⏭", CommandAction::Next)
            .with_keybinding("n")
            .with_category(CommandCategory::Playback),
        Command::new("Previous Track", "⏮", CommandAction::Prev)
            .with_keybinding("p")
            .with_category(CommandCategory::Playback),
        Command::new("Stop", "⏹", CommandAction::Stop)
            .with_keybinding("s")
            .with_category(CommandCategory::Playback),
        Command::new("Seek Forward", "⏩", CommandAction::SeekForward)
            .with_keybinding("l")
            .with_category(CommandCategory::Playback),
        Command::new("Seek Backward", "⏪", CommandAction::SeekBackward)
            .with_keybinding("h")
            .with_category(CommandCategory::Playback),
        Command::new("Volume Up", "🔊", CommandAction::VolumeUp)
            .with_keybinding("+")
            .with_category(CommandCategory::Playback),
        Command::new("Volume Down", "🔉", CommandAction::VolumeDown)
            .with_keybinding("-")
            .with_category(CommandCategory::Playback),
        Command::new("Toggle Mute", "🔇", CommandAction::ToggleMute)
            .with_keybinding("m")
            .with_category(CommandCategory::Playback),
        Command::new("Toggle Shuffle", "🔀", CommandAction::ToggleShuffle)
            .with_keybinding("S")
            .with_category(CommandCategory::Playback),
        Command::new("Cycle Repeat", "🔁", CommandAction::CycleRepeat)
            .with_keybinding("r")
            .with_category(CommandCategory::Playback),
        
        // Navigation
        Command::new("Next Tab", "→", CommandAction::NextTab)
            .with_keybinding("Tab")
            .with_category(CommandCategory::Navigation),
        Command::new("Previous Tab", "←", CommandAction::PrevTab)
            .with_keybinding("Shift+Tab")
            .with_category(CommandCategory::Navigation),
        Command::new("Now Playing", "🎵", CommandAction::SwitchToNowPlaying)
            .with_keybinding("1")
            .with_category(CommandCategory::Navigation),
        Command::new("Library", "📚", CommandAction::SwitchToLibrary)
            .with_keybinding("2")
            .with_category(CommandCategory::Navigation),
        Command::new("Settings", "⚙", CommandAction::SwitchToSettings)
            .with_keybinding("3")
            .with_category(CommandCategory::Navigation),
        
        // Overlays
        Command::new("Queue", "📋", CommandAction::OpenQueue)
            .with_keybinding("Alt+Q")
            .with_category(CommandCategory::Overlays),
        Command::new("YouTube Search", "🔍", CommandAction::OpenYTSearch)
            .with_keybinding("Alt+Y")
            .with_category(CommandCategory::Overlays),
        Command::new("Library Search", "🔎", CommandAction::OpenSearchLibrary)
            .with_keybinding("Alt+F")
            .with_category(CommandCategory::Overlays),
        Command::new("Equalizer", "🎚", CommandAction::OpenEqualizer)
            .with_keybinding("Alt+E")
            .with_category(CommandCategory::Overlays),
        Command::new("About", "ℹ", CommandAction::OpenAbout)
            .with_keybinding("Alt+A")
            .with_category(CommandCategory::Overlays),
        Command::new("Sleep Timer", "⏰", CommandAction::OpenSleepTimer)
            .with_keybinding("Alt+Z")
            .with_category(CommandCategory::Overlays),
        Command::new("Theme Picker", "🎨", CommandAction::OpenThemePicker)
            .with_keybinding("Alt+T")
            .with_category(CommandCategory::Overlays),
        Command::new("Lyrics", "📝", CommandAction::OpenLyrics)
            .with_keybinding("Alt+L")
            .with_category(CommandCategory::Overlays),
        
        // Visualizer
        Command::new("Toggle Visualizer", "📊", CommandAction::ToggleVisualizer)
            .with_keybinding("Ctrl+V")
            .with_category(CommandCategory::Visualizer),
        
        // Settings
        Command::new("Toggle Help Row", "❓", CommandAction::ToggleHelpRow)
            .with_category(CommandCategory::Settings),
        Command::new("Change Progress Style", "📏", CommandAction::ChangeProgressStyle)
            .with_category(CommandCategory::Settings),
        
        // System
        Command::new("Quit", "🚪", CommandAction::Quit)
            .with_keybinding("q")
            .with_category(CommandCategory::System),
    ]
}
```

### 4. Reduce Palette Width

Change from full-width to a reasonable fixed width:

```rust
impl CommandPalette {
    pub fn render(&self, f: &mut Frame, area: Rect) {
        // Calculate width: 50% of terminal, max 60 columns
        let width = (area.width as f32 * 0.5).min(60.0) as u16;
        
        // Center the palette
        let x = (area.width - width) / 2;
        let y = area.y + 2;  // Leave some space at top
        
        let popup_area = Rect {
            x: area.x + x,
            y,
            width,
            height: self.commands.len() as u16 + 4,  // +4 for header, input, footer
        };
        
        // Render with reduced width
        self.render_content(f, popup_area);
    }
    
    fn render_content(&self, f: &mut Frame, area: Rect) {
        let block = Block::default()
            .title(" Command Palette ")
            .borders(Borders::ALL)
            .border_type(BorderType::Plain)
            .style(Style::default().fg(Color::White).bg(Color::Black));
        
        // Render input field
        let input = Paragraph::new(format!("> {}", self.query))
            .style(Style::default().fg(Color::Green));
        
        // Render commands list
        let commands: Vec<ListItem> = self.commands
            .iter()
            .enumerate()
            .map(|(i, cmd)| {
                let style = if i == self.selected {
                    Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                
                let keybinding = cmd.keybinding
                    .as_ref()
                    .map(|k| format!(" [{}]", k))
                    .unwrap_or_default();
                
                ListItem::new(Line::from(vec![
                    Span::styled(&cmd.icon, style),
                    Span::raw(" "),
                    Span::styled(&cmd.name, style),
                    Span::styled(keybinding, Style::default().fg(Color::DarkGray)),
                ]))
            })
            .collect();
        
        let list = List::new(commands)
            .highlight_style(Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))
            .highlight_symbol("> ");
        
        // Layout
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),   // Input
                Constraint::Min(1),     // Commands
                Constraint::Length(2),  // Footer
            ])
            .split(area);
        
        f.render_widget(block, area);
        f.render_widget(input, chunks[0]);
        f.render_widget(list, chunks[1]);
        
        // Footer with key hints
        let footer = Paragraph::new("↑↓ Navigate  Enter Select  Esc Close")
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(footer, chunks[2]);
    }
}
```

### 5. Category Grouping

Group commands by category with headers:

```rust
fn render_grouped_commands(&self, f: &mut Frame, area: Rect) {
    let categories = [
        ("Playback", CommandCategory::Playback),
        ("Navigation", CommandCategory::Navigation),
        ("Overlays", CommandCategory::Overlays),
        ("Visualizer", CommandCategory::Visualizer),
        ("Settings", CommandCategory::Settings),
        ("System", CommandCategory::System),
    ];
    
    let mut items = Vec::new();
    
    for (category_name, category) in &categories {
        let category_commands: Vec<&Command> = self.commands
            .iter()
            .filter(|cmd| &cmd.category == category)
            .collect();
        
        if !category_commands.is_empty() {
            // Add category header
            items.push(ListItem::new(Line::from(Span::styled(
                format!("── {} ──", category_name),
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD),
            ))));
            
            // Add commands
            for cmd in &category_commands {
                items.push(ListItem::new(Line::from(vec![
                    Span::styled(&cmd.icon, Style::default()),
                    Span::raw(" "),
                    Span::styled(&cmd.name, Style::default()),
                ])));
            }
        }
    }
    
    let list = List::new(items);
    f.render_widget(list, area);
}
```

### 6. Search/Filter

Implement fuzzy search for commands:

```rust
impl CommandPalette {
    pub fn filter_commands(&mut self) {
        if self.query.is_empty() {
            self.filtered_commands = self.commands.clone();
        } else {
            let query = self.query.to_lowercase();
            self.filtered_commands = self.commands
                .iter()
                .filter(|cmd| {
                    cmd.name.to_lowercase().contains(&query)
                        || cmd.category.to_string().to_lowercase().contains(&query)
                })
                .cloned()
                .collect();
        }
        
        self.selected = 0;
    }
}
```

## Files to Modify

- `gtm/src/overlay.rs` — Fix command execution, reduce width, add icons
- `gtm/src/command_palette.rs` — New file: CommandPalette struct and rendering
- `gtm/src/app.rs` — Add execute_command method
- `gtm/src/keymap.rs` — Add keybindings for all commands
- `gtm-core/src/ipc.rs` — Ensure all commands are supported

## Implementation Details

### Command Registration

```rust
impl App {
    fn register_commands(&mut self) {
        let commands = get_commands();
        self.command_palette = CommandPalette::new(commands);
    }
}
```

### Command Execution Flow

```
User opens command palette (Alt+P)
  ↓
User types query to filter commands
  ↓
User navigates with ↑↓ keys
  ↓
User presses Enter
  ↓
CommandPalette::execute_selected() called
  ↓
App::execute_command(action) called
  ↓
DaemonReq sent to daemon
  ↓
Command palette closes
```

### Error Handling

```rust
impl App {
    fn execute_command(&mut self, action: CommandAction) -> Result<(), AppError> {
        match action {
            // ... handle commands ...
        }
        .map_err(|e| {
            self.notification = Some(Notification::error(format!("Command failed: {}", e)));
            e
        })
    }
}
```

## Checklist

- [ ] All commands execute properly from command palette
- [ ] Command palette width reduced to 50% (max 60 cols)
- [ ] Icons added to all commands
- [ ] Commands grouped by category
- [ ] Fuzzy search works
- [ ] Keybinding hints shown for each command
- [ ] Command palette closes after execution
- [ ] Error notifications shown for failed commands
- [ ] All keybindings work (Alt+P to open)
- [ ] Navigation with ↑↓ works
- [ ] Enter executes selected command
- [ ] Esc closes palette
- [ ] `cargo check --workspace` passes
- [ ] `cargo test --workspace` passes

## Visual Design

```
┌─────────────────────────────────────┐
│         Command Palette              │
├─────────────────────────────────────┤
│ > search_                            │
├─────────────────────────────────────┤
│ ── Playback ──                       │
│ ▶ Play/Pause [Space]                 │
│ ⏭ Next Track [n]                     │
│ ⏮ Previous Track [p]                 │
│ ⏹ Stop [s]                           │
│ ── Navigation ──                     │
│ → Next Tab [Tab]                     │
│ ← Previous Tab [Shift+Tab]           │
│ ── Overlays ──                       │
│ 📋 Queue [Alt+Q]                     │
│ 🔍 YouTube Search [Alt+Y]            │
├─────────────────────────────────────┤
│ ↑↓ Navigate  Enter Select  Esc Close │
└─────────────────────────────────────┘
```