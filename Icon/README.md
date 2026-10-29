# App icon

`AppIcon.icon` is the [Icon Composer](https://developer.apple.com/icon-composer/)
source. Its filled SVG layers reuse the brand pack's two-moons symbol,
`orbit-simbolo-color-oscuro.svg`, scaled to occupy 52.5% of the
1024-point canvas. The background lives in the document's `fill`; the system
supplies the mask and lighting. The menu-bar artwork is separate.

On a Mac with Xcode 26 or later, `scripts/macos/bundle.sh` compiles the document
with `actool`, checks for a nonempty `Assets.car` and copies it into the bundle
before signing. `CFBundleIconName` selects `AppIcon`. The existing `icon.icns`
and `CFBundleIconFile` remain the fallback for macOS 15; the generated
`AppIcon.icns` is discarded. The icon's compilation target of 26.0 does not
change the app's minimum macOS version of 15.0.

## Mac acceptance

Build the bundle first to validate the source with the real `actool`. Confirm
that `Contents/Resources/Assets.car` is present, `icon.icns` still matches the
checked fallback, and `Contents/Info.plist` names both icons as above.

Render every look with Icon Composer's own `ictool` (not `xcrun ictool`), from
the repository root:

```bash
orbit_ictool="$(xcode-select -p)/../Applications/Icon Composer.app/Contents/Executables/ictool"
orbit_icon_review="$(mktemp -d /tmp/orbit-icon-review.XXXXXX)"
for look in Default Dark TintedLight TintedDark ClearLight ClearDark; do
  "$orbit_ictool" Icon/AppIcon.icon --export-image \
    --output-file "$orbit_icon_review/$look.png" --platform macOS \
    --rendition "$look" --width 256 --height 256 --scale 2
done
for size in 32 16; do
  "$orbit_ictool" Icon/AppIcon.icon --export-image \
    --output-file "$orbit_icon_review/Default-$size.png" --platform macOS \
    --rendition Default --width "$size" --height "$size" --scale 2
done
open "$orbit_icon_review"
```

Inspect every rendering for readable rings, centre and moons, especially at
16 pt. Check the installed app in the Dock and Finder on macOS 26 in each
appearance, and check the legacy icon on macOS 15. Linux packaging tests use
inert compiler doubles and do not establish these visual or compilation results.
