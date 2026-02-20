# Rocas

Rock your download directory (preferably but it's up to you), with Rocas, a simple watcher that moves files to their respective folders based on their extensions.
You can also specify custom folders for specific extensions, and even set up rules for files without extensions.

## Features

- Watch a specified directory for new files.
- Move files to predefined folders based on their extensions.
- Customizable folder paths for different file types.
- Support for files without extensions.
- Cross-platform compatibility (Windows, macOS, Linux).

## Installation

1. Install from GitHub releases or clone the repository and build it yourself.
2. Run the executable and follow the prompts to set up your watch directory and folder paths.

## Usage

1. Launch Rocas and select the directory you want to watch.
2. Configure the folder paths for different file types (e.g., Documents, Images, Videos).
3. Start the watcher, and Rocas will automatically move new files to their respective folders based on their extensions.

## Configuration

Rocas uses a simple configuration file (config.toml) to store the watch directory and folder paths. You can edit this file directly or use the application's interface to update it. The configuration file has the following structure:

```toml
[watcher]
watcher_path = "/home/chiko/Downloads"
recursive = true
interval_millis = 1000
max_depth = 0

[[rules]]
patterns = ["*.pdf", "*.docx", "*.txt"]
destination = "/home/chiko/docs"

[[rules]]
patterns = ["*.jpg", "*.png", "*.gif"]
destination = "/home/chiko/images"
```

Please place the `config.toml` file in the same directory as the Rocas executable.

## Contributing

Contributions are welcome! Please fork the repository and submit a pull request with your changes.
For major changes, please open an issue first to discuss what you would like to change.

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details
