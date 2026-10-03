# Android surface lifecycle, frame pacing and touch cancellation fixes

Vendored from the published `i-slint-backend-android-activity` 1.18.1 crate.
Original licensing and source headers are retained in `LICENSES/` and each file.

Changes are in `androidwindowadapter.rs` and `lib.rs`. Suspend the Skia renderer on
`MainEvent::TerminateWindow` and on `Destroy`. Android may reuse a native window
after Activity recreation; retaining its EGL surface causes
`native_window_api_connect … already connected to another API` / `EGL_BAD_ALLOC`
when the replacement Activity first renders. Release that connection while the
old native window is still valid, rather than relying on eventual Rust teardown.

`./build.sh test android ui` exercises editor restoration and touch input after actual
`Activity.recreate()`, plus background playback and reopening the Activity.
Remove this Cargo patch once the upstream backend handles this lifecycle.

`vsync.rs` drives redraws from actual Choreographer callbacks on a helper looper.
Internal looper messages do not count as frames, and at most one callback is
outstanding. The main loop coalesces input/timer redraws until a callback arrives,
preventing excessive submissions and buffer queue latency. The frame clock stays
active during touches, pending redraws and animations, then parks while idle.
Input draining yields after 2 ms so a replenishing queue cannot starve rendering.
Synchronous `RedrawNeeded` lifecycle events still render immediately; if the
Choreographer is unavailable, the original timer-driven fallback remains.

The application's `MainActivity` requests the highest refresh rate available at
the current resolution through window and surface hints. Android retains control
over power, thermal limits and the user's refresh-rate policy.

Android `ACTION_CANCEL` / `ACTION_OUTSIDE` also dispatch `PointerExited` before
the cancelled touch events. Slint 1.18.1's primary-touch cancellation synthesizes
a mouse release; clearing the item grab first delivers cancellation to TouchArea
instead of committing a swipe or clicking a pressed button. The touch events
still clear Slint's active-finger state. The Android UI suite checks that a
cancelled library swipe leaves the current filter unchanged.
Flickable can also retain a delayed press that Slint flushes on that synthesized
release. Cancelled touches therefore release outside the UI and clear the grab
again afterwards, so a quick swipe cannot briefly open the touched book.

`set_touch_filter` lets the application claim a horizontal library gesture
before Flickable's vertical threshold intercepts a diagonal movement. The filter
receives window coordinates, independent of the animated rows. Consumed events
cancel both the item grab and Slint's active finger, preventing a swipe from
activating a book or its buttons. Unclaimed events retain native scrolling.

`set_ui_scale` applies the application's interface scale on top of Android's
display density and retains physical insets across configuration changes.

`ACTION_SCROLL` routes both mouse wheel axes to Slint's `PointerScrolled`.
The upstream placeholder panicked on external mouse/trackpad scrolling. The
device UI suite injects vertical and horizontal wheel input over the volume
slider and checks that both directions reach the control.

`SlintAndroidJavaHelper.java` also reports IME selection and composition span
changes, honoring nested input batches. These operations do not replace text,
so the upstream editable callback missed them: subsequent hardware input used
the old selection, and finishing composition could leave text uncommitted.
The device UI suite exercises these operations through `InputConnection`.
Batch nesting is tracked per connection, with Android's required begin/end
return values. Closing a connection flushes its outstanding batches, and extra
end calls cannot make the nesting count negative or block future input.
The Java/Rust bridge preserves the IME caret inside composition. Android
composing regions outside the selection remain plain text in Slint (Android
retains their spans), because Slint can only insert preedit at its cursor.
This prevents feedback from rearranging a word when the IME marks another span.
IME feedback is coalesced until Slint has applied both text and selection;
intermediate cursor positions must not rewrite Android's composing spans.
Selection direction is preserved in both directions across JNI: Android's
selection start is the anchor and its end is the moving cursor. Sorting these
offsets or swapping their roles makes Shift+arrow modify the wrong boundary.
The device suite extends IME selections with hardware keys before replacing
the selected text.
Detached input views reject IME edits and suppress pending text notifications.
An old Activity's connection can outlive its window; forwarding its static JNI
callback after recreation otherwise replaces the newly focused editor's text.
The device suite sends a late commit through that old connection after reopening
the restored title field and verifies that its text remains unchanged.
