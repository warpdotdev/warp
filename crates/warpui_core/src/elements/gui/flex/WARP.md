# Flex Element Debugging Guide

This guide helps diagnose and fix common Flex layout panics in WarpUI.

## Quick Reference: Error Messages → Fixes

### Error: `flex contains flexible children but has an infinite constraint along the flex axis`

**Cause**: A `Flex` with `MainAxisSize::Min` (the default) contains an `Expanded` or `Shrinkable` child but has no max constraint along the main axis.

**Fixes** (in order of preference):
1. Remove `Expanded`/`Shrinkable` from the child if growing isn't necessary
2. Add a max constraint to the `Flex` or an ancestor using `ConstrainedBox`:
   - For `Flex::row()`: add `max_width`
   - For `Flex::column()`: add `max_height`
3. If the `Flex` is inside another `Flex`, ensure the parent passes down bounded constraints

### Error: `A flex that should expand to a max space can't be rendered in an infinite max constraint`

**Cause**: A `Flex` has `MainAxisSize::Max` but no ancestor provides a maximum size constraint.

**Fixes**:
1. Remove `.with_main_axis_size(MainAxisSize::Max)` if the Flex doesn't need to fill its parent (use default `MainAxisSize::Min`)
2. Add a `ConstrainedBox` with a max constraint to the `Flex` or an ancestor

## Key Concepts

### Two Types of Children
- **Flexible children** (`Expanded`, `Shrinkable`): Size is calculated by dividing remaining space by flex ratio
- **Non-flexible children**: Laid out using their intrinsic size but still respect the max constraints from their parent `Flex`

### Important Behaviors

1. **`Expanded` only works as direct child of `Flex`** - wrapping in `Container`/`ConstrainedBox` breaks it

2. **`Expanded` doesn't force growth** - unlike CSS `flex-grow`, it only grants the *ability* to grow. Elements like `Text` don't expand by default; wrap in `Align` if needed

3. **`MainAxisSize::Min` + `Expanded` = effectively `MainAxisSize::Max`** - the `Expanded` child will grow to fill available space anyway

4. **Nested `Flex` with `MainAxisSize::Max`** - putting a `Flex` with `MainAxisSize::Max` inside another `Flex` with `MainAxisSize::Max` will cause a layout panic when neither receives a max constraint from an ancestor

## Common Patterns

### Centering horizontally (requires bounded parent)
```rust
Flex::row()
    .with_main_axis_size(MainAxisSize::Max)
    .with_main_axis_alignment(MainAxisAlignment::Center)
    .with_child(element)
    .finish()
```

### Centering vertically (requires bounded parent)
```rust
Flex::row()
    .with_cross_axis_alignment(CrossAxisAlignment::Center)
    .with_child(element)
    .finish()
```

### Spacing groups apart (e.g., left/right aligned items)
```rust
Flex::row()
    .with_child(left_element)
    .with_child(Expanded::new(1.0, Empty::new()))  // spacer
    .with_child(right_element)
    .finish()
```

## Debugging Tips

1. Run with `RUST_BACKTRACE=full` to identify which element(s) cause the panic
2. Check the element hierarchy for unbounded `Flex` containers
3. Trace constraints from root to the failing element - find where max constraint is lost
4. Avoid unnecessary `MainAxisSize::Max` - only use when the `Flex` *must* fill its parent
