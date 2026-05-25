# Shared session QR code — Product Spec
Figma: https://www.figma.com/design/CsBdBW4YoLgSAbr5eSkwV6/House-of-Agents?node-id=7877-43943&t=F8mi5pp4M5ch1kmN-1
## Summary
Users sharing a live Warp session should be able to show a QR code for the same session URL that appears in the live-session sharing dialog. The QR code makes it easy for someone nearby to join from another device without manually copying or typing the link.
## Problem
The live-session sharing dialog already exposes invite controls, access controls, the canonical session URL, and a `Copy link` action. That works for digital sharing, but it is awkward during demos, in-person collaboration, and mobile handoff flows where scanning a QR code is faster than sending a link.
## Goals
- Add a QR-code entry point to the existing live-session sharing dialog.
- Add a `View QR code` call to action to the `Remote control link copied.` toast shown immediately after a shared session starts.
- Ensure the QR code represents exactly the same URL shown in the dialog footer.
- Let users copy the session link from the QR-code view or download the QR code as a PNG.
- Preserve the existing `Copy link` behavior and sharing access controls.
## Non-goals
- Changing shared-session permission semantics or default access levels.
- Creating a separate link format for QR codes.
- Adding QR codes for non-session shareable objects such as notebooks, workflows, Warp Drive objects, or AI conversations.
- Adding analytics, expiration, or tracking parameters to the QR URL beyond what the canonical session URL already contains.
## Behavior
1. When Warp starts a shared terminal session and copies the remote-control link automatically, the toast reads `Remote control link copied.` and includes a `View QR code` link.
2. Activating `View QR code` opens the QR-code view for that newly-started shared session directly in the pane-header sharing overlay. Warp does not require the user to open the access-management panel first. Once that overlay opens, the toast may expire or be dismissed independently, but that must not dismiss, navigate away from, or otherwise change the QR-code view.
3. The QR-code view replaces the sharing dialog's contents inside the same compact overlay. It must not open a separate full-height panel or stretch the overlay to fill the workspace height.
4. If the live-session sharing overlay is already open when the toast action is activated, Warp transitions that overlay into the QR-code view instead of creating a second overlay.
5. Pressing Back from a toast-opened QR-code view returns to the live-session sharing dialog for the same session.
6. When the live-session sharing dialog is open for a shared terminal session and a canonical session URL is available, the footer shows three controls in this order:
   - A read-only, truncated text field containing the canonical session URL.
   - An icon-only QR-code button.
   - The existing `Copy link` button.
7. The QR-code button is visually distinct and immediately adjacent to the link field and `Copy link` button, matching the Figma layout. It must read as its own bordered button rather than visually merging into the URL field or disappearing into the footer row, and it does not replace or change `Copy link`.
8. The QR-code button shows a tooltip equivalent to `Show QR code`.
9. Activating the QR-code button opens a QR-code view in the same sharing overlay. The underlying shared session continues uninterrupted.
10. The QR-code view header contains:
   - A back arrow on the left.
   - The title `Share session QR code`.
   - An `ESC` keyboard hint.
   - A close button.
11. Pressing the back arrow returns to the live-session sharing dialog with the previous invite text, access menus, link state, and scroll position preserved as much as the existing dialog architecture allows.
12. Pressing Escape or the close button dismisses the sharing overlay, consistent with the existing sharing dialog dismiss behavior.
13. The QR-code view renders a scannable QR code for the exact URL shown in the live-session sharing dialog footer at the time the QR view is rendered.
14. The QR code is centered in a square card matching the mock:
   - Dialog width about 400px.
   - QR card about 192px square.
   - QR image about 160px square.
   - High-contrast black modules on a white background with a sufficient quiet zone.
15. QR code rendering is intentionally not theme-colored. The code must stay black-on-white in both dark and light themes so phone cameras can scan it reliably.
16. If the session URL changes while the QR-code view is open, the QR image updates to encode the latest canonical URL. This includes channel-specific URL changes and preview/staging URL behavior already handled by the session-link generator.
17. If the session ends while the QR-code view is open, the QR view keeps encoding the same URL that the sharing dialog would copy for the session. If Warp can no longer provide a session URL, the QR view closes or shows a non-scannable error state instead of displaying stale or empty data.
18. Scanning the QR code has the same result as opening the visible session URL directly. Authorization is still enforced by the existing shared-session access controls; QR generation does not grant access by itself.
19. The copy icon in the QR-code view copies the same plain session URL as the sharing dialog's existing `Copy link` button, across all supported platforms.
20. When the QR-code view copy action succeeds, Warp shows the existing link-copy success feedback.
21. The QR-code view copy action does not attempt to write QR image data to the clipboard.
22. The download icon in the QR-code view opens the platform save-file flow for a PNG file. The default filename is recognizable as a Warp session QR code and includes the session id when available.
23. If the user cancels the save-file flow, Warp leaves the QR-code view open and does not show an error.
24. If PNG generation or file writing fails, Warp leaves the QR-code view open and shows a concise failure toast.
25. The existing `Copy link` button in the live-session sharing dialog continues to copy the plain session URL and show the existing link-copy success feedback.
26. The QR-code view's copy and download icon buttons show tooltips equivalent to `Copy link` and `Download QR code`.
27. Multiple shared sessions can each open their own sharing dialog and QR-code view. Each QR code always encodes the URL for that dialog's session target.
28. The QR code must not encode additional sensitive data beyond the canonical session URL that the user can already copy from the sharing dialog.
