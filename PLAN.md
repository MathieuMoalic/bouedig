# Shopping List Fix Plan

## Issues to Fix

1. **Dropdown always open** — The suggestions dropdown should only appear when the input is focused AND has typed text, not by default.
2. **Fuzzy matching broken** — "appel" should match "apple". Replace the current substring-based matching with Levenshtein distance (max 3 edits).
3. **Checkboxes should be X delete buttons** — Replace the checkbox with an "X" button that removes the item.
4. **Items re-appear after adding new item** — When an item is ticked (removed) and a new item is added, the old item comes back. This is because the `refresh()` call after adding fetches the full list from the server, but the local state was already modified by the tick. Need to ensure the local state update is applied correctly.

## Files to Modify

- `frontend-web/src/main.rs` — Grocery component, GroceryRow component, suggestions dropdown
- `frontend-web/assets/style.css` — styles for X button, suggestions dropdown
- `frontend-android/src/main.rs` — same changes for Android frontend
- `frontend-android/assets/style.css` — same styles for Android

## Implementation Steps

### 1. Fix dropdown visibility
- Add a `focused` signal to the Grocery component
- Only show suggestions when `focused && !suggestions.is_empty()`
- Use `onfocus` and `onblur` on the input to toggle the focused state
- Show suggestions only when input is focused AND has non-empty typed text

### 2. Implement Levenshtein distance
- Add `levenshtein_distance(a: &str, b: &str) -> usize` function
- Update `rank_suggestions` to use Levenshtein distance (max 3 edits)
- Filter out matches with distance > 3
- Sort by distance ascending

### 3. Replace checkbox with X button
- Remove the `<input type="checkbox">` from GroceryRow
- Add a `<button class="remove-btn">` with an X icon (use IconX or inline SVG)
- Style the X button appropriately (small, round, danger color)
- Keep the same onclick handler that removes the item

### 4. Fix items re-appearing
- The issue: When adding an item, `refresh()` is called which fetches from server. But the server has the ticked item still (because we only removed it locally, and the PATCH may have failed or the server still has it).
- Actually, looking at the code: the tick handler sends PATCH with `bought: true` which deletes from server. If successful, the item is removed locally. But then `refresh()` after adding fetches from server again — if the server deletion succeeded, the item should be gone.
- Wait, the issue might be that `refresh()` is called in the add handler, which re-fetches the full list. If the server deletion was successful, the item should be gone. But maybe there's a race condition or the server hasn't processed the deletion yet.
- Actually, looking more carefully: the `refresh()` after adding fetches from server. The server should have the item deleted. But maybe the issue is that the local state was modified by the tick handler (removing the item), and then `refresh()` overwrites it with the server state. If the server state still has the item (because deletion failed or was slow), the item comes back.
- Better approach: Don't call `refresh()` after adding. Instead, just add the new item to the local state. This way, the local state is the source of truth and won't be overwritten by server state.
- Or: Make the tick handler wait for the server response before removing locally, and then the add handler's refresh should see the updated server state.
- Actually, the simplest fix: In the add handler, after POST succeeds, just push the new item to the local state instead of calling `refresh()`. This avoids re-fetching from server.

### 5. Update Android frontend
- Apply the same changes to `frontend-android/src/main.rs`
- Apply the same CSS changes to `frontend-android/assets/style.css`

## Test Plan

### Unit Tests
- Add tests for `levenshtein_distance` in `frontend-web/src/main.rs`
- Add tests for `rank_suggestions` in `frontend-web/src/main.rs`

### E2E Tests
- Check that suggestions dropdown appears only when input is focused
- Check that fuzzy matching works (appel -> apple)
- Check that X button removes item
- Check that adding a new item after removing one doesn't bring back the old item