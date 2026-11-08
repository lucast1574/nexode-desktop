# Update channels

Nexode uses the **stable** update channel by default. Existing `main` channel users keep their source-build update behavior.

The installed binary must include preview-channel support. If `--channel` is not recognized, first update to a stable version that provides it, or install a preview release artifact manually. Older stable binaries only query the latest stable GitHub release and cannot discover previews themselves.

To opt in to the v1 preview and immediately check for a preview update:

```sh
nexode update --channel preview
```

This saves `update_channel = "preview"` in the `[features]` section of your Nexode config. Later explicit and automatic checks follow that preference. Merely running a preview binary does not opt you in. Existing settings controlling automatic update checks and installation still apply.

Preview discovery considers only published, non-draft GitHub prereleases named exactly `v1.0.0-preview.N`, where `N` is a positive integer without leading zeroes. It selects the highest preview number with an available asset for your platform and `SHA256SUMS`. Downloads must pass checksum verification. Other prereleases, incomplete uploads, and stable releases are not preview candidates. Preview checks do not downgrade an installed version.

To leave the preview channel:

```sh
nexode update --channel stable
```

This persists the stable preference and explicitly permits installing the latest stable release even when its version is lower than the running preview. Normal automatic stable checks never downgrade. If a channel switch cannot download an update, the preference is still saved for the next check. `nexode update` without `--channel` does not change your preference.

You can also set the config directly:

```toml
[features]
update_channel = "preview" # stable (default), preview, or main
```

`NEXODE_UPDATE_CHANNEL` overrides the saved preference for normal checks. An explicit `nexode update --channel ...` takes priority for that invocation and saves the requested preference without persisting unrelated environment overrides.

## Preview binary identity

Release builders must supply the full preview identity, not just the numeric Cargo package version:

```sh
NEXODE_RELEASE_BUILD=1
NEXODE_BUILD_SEMVER=1.0.0-preview.N
NEXODE_BUILD_GIT_TAG=v1.0.0-preview.N
```

Continue providing the existing build git hash and date inputs as appropriate. The updater compares semantic version identity, including prerelease ordering, while ignoring SemVer build metadata (`+...`). This prevents repeatedly reinstalling the same preview and orders preview 10 after preview 9.
