# Inno Setup installer script

## What is `windows-installer.iss`?

On Windows, programs are conventionally installed using an installer, also known as an installation wizard.
The installer is a single executable that takes care of:
* Creating a directory to store the program's files
* Downloading assets
* Initializing registry entries
* Creating a desktop icon
* ... and more, depending on the application's needs.


`windows-installer.iss` is an **Inno Setup script**:
a configuration file for building a Warp installer.
The Inno Setup Compiler takes a script file and generates an installer executable.
This is roughly equivalent to the bundling process on MacOS.


## How to edit the installer

See the Inno Setup documentation: [Inno Setup Help](https://jrsoftware.org/ishelp/).
This script can be edited manually using any code editor.
However, it requires the Inno Setup compiler to be turned into a `.exe` file.


## How to compile this installer

First, ensure you've set up your environment.
* Download and install the [Inno Setup Compiler](https://jrsoftware.org/isdl.php).
* Run `cargo build` to ensure the installer uses the latest version of Warp.

### Option 1: Use the CLI
1. Add the Inno Setup Command-line Compiler executable to your shell path.
By default, it is located at `C:\Program Files (x86)\Inno Setup 6\ISCC.exe`.
2. Compile the installer:
```shell
iscc .\script\windows\windows-installer.iss
```
3. Run the generated executable:
```shell
.\script\windows\Output\Warp-Windows-Setup.exe
```

The script begins with a series of preprocessor definitions.
From the command line, use the `/D` flag to emulate preprocessor definitions
and override the hardcoded defaults.
Usage: `iscc <script path> /D<name>[=<value>]`

The following constants can be overwritten:
* `MyAppVersion` (default: `0.1.0`)
* `MyAppExeName` (default: `warp.exe`)
* `ReleaseChannel` (default: `dev`)
* `TargetProfileDir` (default: `debug`)

### Option 2: Use the GUI
1. Open the Inno Setup application and select this script.
2. Click the "compile" button. This will generate an installer executable in a directory called `Output` at the same level as this script.
2. To run the installer, click the "run" button in Inno Setup.


## Using icons

Windows has its own icon file format that bundles together multiple icon sizes.
App icons are located in `app/channels/<channel_name>/icon/no-padding`.
The `.ico` files are generated using imagemagick:

```shell
convert 16x16.png 32x32.png 48x48.png 64x64.png 256x256.png icon.ico
```

Note that sizes above 256x256 are not supported.
See the [Inno Setup docs](https://jrsoftware.org/ishelp/index.php?topic=setup_setupiconfile).
