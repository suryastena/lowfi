# UI Component System Integration Guide

## Overview
This guide explains how to integrate the new modular UI component system into your lowfi music player. The new system provides a flexible, trait-based architecture that makes it easy to add, remove, and customize UI components.

## Key Features

### 1. **Component-Based Architecture**
- All UI elements are now components implementing the `UIComponent` trait
- Components can be composed together using layout containers
- Easy to add new custom components without modifying core UI code

### 2. **Flexible Layouts**
- `VStack` for vertical layouts
- `HStack` for horizontal layouts (in custom_components_example.rs)
- `Container` trait for custom layout implementations

### 3. **Dynamic Components**
- `DynamicComponent` allows switching between different UI states
- Perfect for toggling between progress bar and volume bar

### 4. **Event System**
- Components can handle events through `handle_event()`
- Events bubble up through the component tree
- Custom events supported for extensibility

## File Structure

```
src/player/ui/
├── components.rs          # Core component system and basic components
├── ui_modular.rs         # Main UI implementation using components
└── custom_components.rs  # Example custom components
```

## Integration Steps

### Step 1: Backup Current Implementation
```bash
cp src/player/ui.rs src/player/ui_legacy.rs
cp src/player/ui/components.rs src/player/ui/components_legacy.rs
```

### Step 2: Replace Core Files

1. Replace `src/player/ui/components.rs` with `ui_component_system.rs`
2. Replace `src/player/ui.rs` with `ui_modular.rs`
3. Add `custom_components_example.rs` as `src/player/ui/custom_components.rs`

### Step 3: Update Imports

In `src/player.rs`, the imports should remain mostly the same:
```rust
pub mod ui;
use ui::{UIEvent};
```

### Step 4: Minimal Code Changes

The main integration points are designed to be drop-in replacements. The public API remains the same:
- `ui::start()` function signature unchanged
- `UIEvent` enum remains compatible
- `flash_audio()` function still available

## Creating Custom Components

### Basic Component Template
```rust
pub struct MyCustomComponent {
    // Component state
    data: String,
}

impl MyCustomComponent {
    pub fn new(data: String) -> Self {
        Self { data }
    }
}

impl UIComponent for MyCustomComponent {
    fn render(&self, context: &RenderContext) -> String {
        // Return the string representation of your component
        format!("Custom: {}", self.data)
    }
    
    fn handle_event(&mut self, event: ComponentEvent) -> EventResult {
        match event {
            ComponentEvent::Custom(msg) => {
                // Handle custom events
                EventResult::Consumed
            }
            _ => EventResult::Ignored
        }
    }
}
```

### Adding to Layout
```rust
let mut layout = VStack::new();
layout.add_child(Box::new(MyCustomComponent::new("Hello".to_string())));
```

## Examples of New Components You Can Add

### 1. **Bitrate Display**
```rust
pub struct BitrateDisplay {
    bitrate: u32,
}

impl UIComponent for BitrateDisplay {
    fn render(&self, _context: &RenderContext) -> String {
        format!("{}kbps", self.bitrate)
    }
}
```

### 2. **Sleep Timer**
```rust
pub struct SleepTimer {
    remaining: Option<Duration>,
}

impl UIComponent for SleepTimer {
    fn render(&self, _context: &RenderContext) -> String {
        match self.remaining {
            Some(duration) => format!("Sleep: {}", format_duration(&duration)),
            None => "Sleep: Off".to_string(),
        }
    }
}
```

### 3. **Track Queue Display**
```rust
pub struct QueueDisplay {
    queue_size: usize,
}

impl UIComponent for QueueDisplay {
    fn render(&self, _context: &RenderContext) -> String {
        format!("Queue: {} tracks", self.queue_size)
    }
}
```

### 4. **Repeat/Shuffle Indicators**
```rust
pub struct PlaybackMode {
    repeat: bool,
    shuffle: bool,
}

impl UIComponent for PlaybackMode {
    fn render(&self, _context: &RenderContext) -> String {
        let mut modes = Vec::new();
        if self.repeat { modes.push("🔁"); }
        if self.shuffle { modes.push("🔀"); }
        modes.join(" ")
    }
}
```

## Customizing the Default Layout

In `ui_modular.rs`, modify the `UIManager::new()` method:

```rust
impl UIManager {
    pub fn new(player: Arc<Player>, width: usize, borderless: bool, minimalist: bool) -> Self {
        let mut window = ComponentWindow::new(width, borderless);
        
        // Create custom layout
        let mut layout = VStack::new();
        
        // Add custom components
        layout.add_child(Box::new(StatusBar::new()));
        layout.add_child(Box::new(BitrateDisplay::new()));  // Custom!
        layout.add_child(Box::new(ProgressBar::new()));
        layout.add_child(Box::new(QueueDisplay::new()));    // Custom!
        
        if !minimalist {
            layout.add_child(Box::new(ControlBar::new()));
        }
        
        window.set_root(Box::new(layout));
        
        // ... rest of initialization
    }
}
```

## Advanced Features

### Conditional Rendering
```rust
impl UIComponent for ConditionalComponent {
    fn is_visible(&self) -> bool {
        // Return false to hide the component
        self.should_show
    }
}
```

### Responsive Components
```rust
impl UIComponent for ResponsiveComponent {
    fn render(&self, context: &RenderContext) -> String {
        if context.width < 40 {
            // Compact view
            self.render_compact()
        } else {
            // Full view
            self.render_full()
        }
    }
}
```

### Animated Components
```rust
pub struct AnimatedSpinner {
    frames: Vec<&'static str>,
    current_frame: usize,
}

impl AnimatedSpinner {
    pub fn tick(&mut self) {
        self.current_frame = (self.current_frame + 1) % self.frames.len();
    }
}

impl UIComponent for AnimatedSpinner {
    fn render(&self, _context: &RenderContext) -> String {
        self.frames[self.current_frame].to_string()
    }
}
```

## Testing Your Components

Create unit tests for your components:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_my_component() {
        let component = MyCustomComponent::new("test".to_string());
        
        let context = RenderContext {
            width: 80,
            playback_state: PlaybackState::Playing,
            // ... set up context
        };
        
        let output = component.render(&context);
        assert_eq!(output, "Custom: test");
    }
}
```

## Performance Considerations

1. **Minimize Allocations**: Reuse strings where possible
2. **Lazy Rendering**: Only render visible components
3. **Event Filtering**: Components should quickly return `Ignored` for unhandled events
4. **State Management**: Keep component state minimal and efficient

## Migration Checklist

- [ ] Backup existing UI implementation
- [ ] Copy new component system files
- [ ] Update imports in player.rs
- [ ] Test basic UI functionality
- [ ] Add custom components as needed
- [ ] Test event handling
- [ ] Verify performance is acceptable
- [ ] Update documentation

## Troubleshooting

### Component Not Showing
- Check `is_visible()` returns true
- Verify component is added to a container
- Check container is part of the root component tree

### Events Not Handled
- Ensure `handle_event()` is implemented
- Check event type matches what's expected
- Verify event is being sent from input handler

### Layout Issues
- Check `min_width()` is reasonable
- Verify `RenderContext.width` is set correctly
- Test with different terminal widths

## Benefits of the New System

1. **Modularity**: Each component is self-contained
2. **Reusability**: Components can be used in multiple places
3. **Testability**: Components can be unit tested in isolation
4. **Extensibility**: Easy to add new components without modifying core code
5. **Maintainability**: Clear separation of concerns
6. **Type Safety**: Trait-based system provides compile-time guarantees

## Future Enhancements

Consider these potential improvements:

1. **Theme System**: Add theming support to components
2. **Animation Framework**: Built-in animation support
3. **Layout Constraints**: More sophisticated layout algorithms
4. **Component Library**: Pre-built component collection
5. **Hot Reload**: Dynamic component reloading during development
6. **Serialization**: Save/load UI layouts from config files

## Support

For questions or issues with the component system, consider:
- Creating detailed component documentation
- Adding more examples in custom_components.rs
- Setting up component playground for testing
- Creating visual component gallery
