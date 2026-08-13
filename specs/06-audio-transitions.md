# 06 — Better Audio Transitions

## Goal

Research and implement creative audio transition options that users can pick from. Provide multiple transition styles beyond basic crossfade.

## Current State

Basic crossfade functionality exists in `gtm-audio/src/mixer.rs` with easing functions. Need to expand with more creative transition options.

## Research: Audio Transition Techniques

### 1. Crossfade Variants

#### Linear Crossfade
```
Track A: ████████████▓▓▓▓▓▓░░░░░░░░░░░░
Track B: ░░░░░░░░░░░░▓▓▓▓▓▓████████████
```
- Simple linear volume interpolation
- Most common, works well for most music

#### Equal Power Crossfade
```
Track A: ████████████▓▓▓▓░░░░░░░░░░░░░░
Track B: ░░░░░░░░░░░░▓▓▓▓████████████
```
- Maintains constant power during transition
- Prevents volume dip in middle
- Better for professional audio

#### Exponential Crossfade
```
Track A: ████████████▓▓▓░░░░░░░░░░░░░░░
Track B: ░░░░░░░░░░░░░▓▓▓████████████
```
- Track A fades out quickly at end
- Track B fades in slowly at start
- Good for electronic/dance music

### 2. DJ-Style Transitions

#### Beat Match Crossfade
- Analyze BPM of both tracks
- Align beat grids
- Crossfade on beat boundaries
- Requires BPM detection

#### Echo Out / Delay Fade
```
Track A: ████████████░░░░░░░░░░░░░░░░░░
         (echo tail) ~~~  ~~~  ~~~
Track B: ░░░░░░░░░░░░████████████████
```
- Add echo/delay effect to outgoing track
- Creates spacey transition
- Good for ambient/progressive music

#### Filter Sweep
```
Track A: ████████████░░░░░░░░░░░░░░░░░░
         (low-pass filter sweep)
Track B: ░░░░░░░░░░░░████████████████
```
- Apply low-pass filter to outgoing track
- Apply high-pass filter to incoming track
- Sweep frequencies during transition
- Common in DJ mixes

### 3. Creative Transforms

#### Reverb Tail
```
Track A: ████████████░░░░░░░░░░░░░░░░░░
         (reverb tail) ~~~~~~~~
Track B: ░░░░░░░░░░░░░░░░████████████
```
- Add reverb to outgoing track
- Let it decay naturally
- Fade in new track after reverb tail

#### Vinyl Stop Effect
```
Track A: ████████████▓▓▓░░░
         (pitch down + stop)
Track B: ░░░░░░░░░░░░░████████████████
```
- Slow down outgoing track (like vinyl stopping)
- Pitch drops with speed
- Dramatic transition for emphasis

#### Tape Stop Effect
```
Track A: ████████████▓▓▓▓░░░
         (tape stop effect)
Track B: ░░░░░░░░░░░░░░████████████
```
- Similar to vinyl stop but with tape character
- More pronounced pitch drop
- Good for hip-hop/rap transitions

### 4. Silence-Based Transitions

#### Hard Cut
```
Track A: ████████████|░░░░░░░░░░░░░░░░░
Track B: ░░░░░░░░░░░░|████████████████
```
- Immediate switch at track boundary
- No overlap
- Good for live recordings/podcasts

#### Gap with Breath
```
Track A: ████████████|░░░|░░░░░░░░░░░░░
Track B: ░░░░░░░░░░░░|░░░|████████████
```
- Brief silence between tracks
- Allows listener to "breathe"
- Good for classical/jazz

#### Silence with Reverb
```
Track A: ████████████|░░░ ~~~ ~~~
Track B: ░░░░░░░░░░░░░░░░|████████████
```
- Track ends with reverb tail in silence
- Creates natural decay
- Good for acoustic music

### 5. Volume-Based Transitions

#### Volume Ducking
```
Track A: ████████████▓▓▓▓▓▓░░░░░░░░░░░░
Track B: ░░░░░░░░░░░░████████████████
         (volume boost during transition)
```
- Outgoing track volume reduced
- Incoming track volume boosted slightly
- Maintains perceived loudness

#### Volume Ramp
```
Track A: ████████████▓▓▓░░░░░░░░░░░░░░░
Track B: ░░░░░░░░░░░░░▓▓▓████████████
```
- Smooth volume ramp up/down
- More natural than linear crossfade

## Required Changes

### 1. Create TransitionStyle Enum

Add to `gtm-audio/src/mixer.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransitionStyle {
    // Crossfade variants
    LinearCrossfade,
    EqualPowerCrossfade,
    ExponentialCrossfade,
    
    // DJ-style
    EchoOut,
    FilterSweep,
    
    // Creative
    ReverbTail,
    VinylStop,
    TapeStop,
    
    // Silence-based
    HardCut,
    GapWithBreath,
    SilenceWithReverb,
    
    // Volume-based
    VolumeDucking,
    VolumeRamp,
    
    // No transition
    None,
}

impl TransitionStyle {
    pub fn name(&self) -> &str {
        match self {
            Self::LinearCrossfade => "Linear Crossfade",
            Self::EqualPowerCrossfade => "Equal Power Crossfade",
            Self::ExponentialCrossfade => "Exponential Crossfade",
            Self::EchoOut => "Echo Out",
            Self::FilterSweep => "Filter Sweep",
            Self::ReverbTail => "Reverb Tail",
            Self::VinylStop => "Vinyl Stop",
            Self::TapeStop => "Tape Stop",
            Self::HardCut => "Hard Cut",
            Self::GapWithBreath => "Gap with Breath",
            Self::SilenceWithReverb => "Silence with Reverb",
            Self::VolumeDucking => "Volume Ducking",
            Self::VolumeRamp => "Volume Ramp",
            Self::None => "None",
        }
    }
    
    pub fn description(&self) -> &str {
        match self {
            Self::LinearCrossfade => "Simple linear volume interpolation",
            Self::EqualPowerCrossfade => "Maintains constant power during transition",
            Self::ExponentialCrossfade => "Track A fades quickly, B fades slowly",
            Self::EchoOut => "Add echo effect to outgoing track",
            Self::FilterSweep => "Sweep frequencies during transition",
            Self::ReverbTail => "Add reverb to outgoing track, let it decay",
            Self::VinylStop => "Slow down outgoing track like vinyl stopping",
            Self::TapeStop => "Tape stop effect with pronounced pitch drop",
            Self::HardCut => "Immediate switch at track boundary",
            Self::GapWithBreath => "Brief silence between tracks",
            Self::SilenceWithReverb => "Track ends with reverb tail in silence",
            Self::VolumeDucking => "Outgoing volume reduced, incoming boosted",
            Self::VolumeRamp => "Smooth volume ramp up/down",
            Self::None => "No transition",
        }
    }
}
```

### 2. Implement Transition Effects

```rust
impl Mixer {
    pub fn apply_transition(
        &mut self,
        style: TransitionStyle,
        duration_secs: f64,
        sample_rate: u32,
    ) {
        match style {
            TransitionStyle::LinearCrossfade => {
                self.apply_linear_crossfade(duration_secs, sample_rate);
            }
            TransitionStyle::EqualPowerCrossfade => {
                self.apply_equal_power_crossfade(duration_secs, sample_rate);
            }
            TransitionStyle::ExponentialCrossfade => {
                self.apply_exponential_crossfade(duration_secs, sample_rate);
            }
            TransitionStyle::EchoOut => {
                self.apply_echo_out(duration_secs, sample_rate);
            }
            TransitionStyle::FilterSweep => {
                self.apply_filter_sweep(duration_secs, sample_rate);
            }
            TransitionStyle::ReverbTail => {
                self.apply_reverb_tail(duration_secs, sample_rate);
            }
            TransitionStyle::VinylStop => {
                self.apply_vinyl_stop(duration_secs, sample_rate);
            }
            TransitionStyle::TapeStop => {
                self.apply_tape_stop(duration_secs, sample_rate);
            }
            TransitionStyle::HardCut => {
                // No processing needed
            }
            TransitionStyle::GapWithBreath => {
                self.apply_gap_with_breath(duration_secs, sample_rate);
            }
            TransitionStyle::SilenceWithReverb => {
                self.apply_silence_with_reverb(duration_secs, sample_rate);
            }
            TransitionStyle::VolumeDucking => {
                self.apply_volume_ducking(duration_secs, sample_rate);
            }
            TransitionStyle::VolumeRamp => {
                self.apply_volume_ramp(duration_secs, sample_rate);
            }
            TransitionStyle::None => {
                // No transition
            }
        }
    }
    
    fn apply_linear_crossfade(&mut self, duration: f64, sample_rate: u32) {
        let samples = (duration * sample_rate as f64) as usize;
        
        for i in 0..samples {
            let progress = i as f64 / samples as f64;
            let fade_out = 1.0 - progress;
            let fade_in = progress;
            
            // Apply to audio buffers
            self.outgoing_volume = fade_out;
            self.incoming_volume = fade_in;
        }
    }
    
    fn apply_equal_power_crossfade(&mut self, duration: f64, sample_rate: u32) {
        let samples = (duration * sample_rate as f64) as usize;
        
        for i in 0..samples {
            let progress = i as f64 / samples as f64;
            // Equal power: sin^2 + cos^2 = 1
            let fade_out = (progress * std::f64::consts::FRAC_PI_2).cos();
            let fade_in = (progress * std::f64::consts::FRAC_PI_2).sin();
            
            self.outgoing_volume = fade_out;
            self.incoming_volume = fade_in;
        }
    }
    
    fn apply_vinyl_stop(&mut self, duration: f64, sample_rate: u32) {
        let samples = (duration * sample_rate as f64) as usize;
        
        for i in 0..samples {
            let progress = i as f64 / samples as f64;
            // Slow down and pitch drop
            let speed = 1.0 - progress;
            let pitch = speed;
            
            self.outgoing_speed = speed;
            self.outgoing_pitch = pitch;
            self.outgoing_volume = 1.0 - progress;
        }
    }
    
    fn apply_tape_stop(&mut self, duration: f64, sample_rate: u32) {
        let samples = (duration * sample_rate as f64) as usize;
        
        for i in 0..samples {
            let progress = i as f64 / samples as f64;
            // More pronounced pitch drop than vinyl
            let speed = (1.0 - progress).powf(2.0);
            let pitch = speed.powf(1.5);
            
            self.outgoing_speed = speed;
            self.outgoing_pitch = pitch;
            self.outgoing_volume = 1.0 - progress;
        }
    }
    
    fn apply_filter_sweep(&mut self, duration: f64, sample_rate: u32) {
        let samples = (duration * sample_rate as f64) as usize;
        
        for i in 0..samples {
            let progress = i as f64 / samples as f64;
            // Low-pass filter sweep on outgoing
            let cutoff = 20000.0 * (1.0 - progress);
            // High-pass filter sweep on incoming
            let incoming_cutoff = 20.0 + (progress * 19980.0);
            
            self.outgoing_filter_cutoff = cutoff;
            self.incoming_filter_cutoff = incoming_cutoff;
        }
    }
}
```

### 3. Add Settings Option

Add to Settings tab:

```
┌─ SETTINGS ──────────────────────────────────────┐
│  ♫ Audio       Transition Style  [ LinearCrossfade ▶ ] │
│  ▶ Appearance  Crossfade Duration [ 3.0s      ] │
└─────────────────────────────────────────────────┘
```

### 4. Configuration

Add to config file (`~/.config/gtom/config.toml`):

```toml
[audio]
transition_style = "EqualPowerCrossfade"
crossfade_duration = 3.0  # seconds
```

### 5. Transition Preview

Add preview in Settings tab:

```rust
fn render_transition_preview(&self, f: &mut Frame, area: Rect) {
    let preview_width = 40;
    let preview_height = 5;
    
    let block = Block::default()
        .title(" Transition Preview ")
        .borders(Borders::ALL);
    
    // Render visual representation of transition
    let lines = match self.selected_style {
        TransitionStyle::LinearCrossfade => vec![
            "Track A: ████████████▓▓▓▓▓▓░░░░░░░░░░░░",
            "Track B: ░░░░░░░░░░░░▓▓▓▓▓▓████████████",
        ],
        TransitionStyle::EqualPowerCrossfade => vec![
            "Track A: ████████████▓▓▓▓░░░░░░░░░░░░░░",
            "Track B: ░░░░░░░░░░░░▓▓▓▓████████████",
        ],
        // ... other styles
    };
    
    let paragraph = Paragraph::new(lines.join("\n"))
        .block(block);
    
    f.render_widget(paragraph, area);
}
```

## Files to Modify

- `gtm-audio/src/mixer.rs` — Add TransitionStyle enum and implementations
- `gtm-audio/src/lib.rs` — Export new types
- `gtm-core/src/ipc.rs` — Add transition style to IPC commands
- `gtmd/src/daemon.rs` — Apply transition style on track change
- `gtm/src/app.rs` — Add transition style to app state
- `gtm/src/overlay.rs` — Add transition style selector to Settings
- `gtm/src/settings.rs` — Add transition settings UI
- `~/.config/gtom/config.toml` — Add transition config options

## Implementation Details

### Audio Processing Pipeline

```
Incoming Audio → Filter → Volume → Mix → Output
                    ↑
                    │
Outgoing Audio → Filter → Volume → Mix → Output
                    ↑
                    │
              Transition Effects
```

### Real-Time Processing

```rust
impl Mixer {
    fn process_transition(&mut self, outgoing: &[f32], incoming: &[f32]) -> Vec<f32> {
        let mut output = Vec::with_capacity(outgoing.len());
        
        for (out_sample, in_sample) in outgoing.iter().zip(incoming.iter()) {
            let out = *out_sample * self.outgoing_volume as f32;
            let in_ = *in_sample * self.incoming_volume as f32;
            output.push(out + in_);
        }
        
        output
    }
}
```

### State Management

```rust
pub struct TransitionState {
    pub style: TransitionStyle,
    pub duration: f64,
    pub progress: f64,
    pub is_active: bool,
    pub outgoing_volume: f64,
    pub incoming_volume: f64,
    pub outgoing_speed: f64,
    pub incoming_speed: f64,
    pub outgoing_pitch: f64,
    pub incoming_pitch: f64,
}
```

## Checklist

- [ ] TransitionStyle enum created with all variants
- [ ] Linear crossfade implemented
- [ ] Equal power crossfade implemented
- [ ] Exponential crossfade implemented
- [ ] Echo out effect implemented
- [ ] Filter sweep implemented
- [ ] Reverb tail implemented
- [ ] Vinyl stop effect implemented
- [ ] Tape stop effect implemented
- [ ] Hard cut implemented
- [ ] Gap with breath implemented
- [ ] Silence with reverb implemented
- [ ] Volume ducking implemented
- [ ] Volume ramp implemented
- [ ] Settings option added
- [ ] Config file option added
- [ ] Transition preview in Settings
- [ ] All transitions work with crossfade duration
- [ ] `cargo check --workspace` passes
- [ ] `cargo test --workspace` passes

## Performance Considerations

- Pre-compute transition curves when style changes
- Use lookup tables for common transitions
- Minimize allocations during real-time processing
- Consider SIMD for audio processing if needed

## Testing Strategy

1. **Unit Tests**: Test each transition curve calculation
2. **Integration Tests**: Test transitions with real audio
3. **A/B Testing**: Compare transitions with same audio
4. **Performance Tests**: Measure CPU usage during transitions
5. **Subjective Testing**: Get user feedback on transition quality