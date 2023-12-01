# Syncbox

## Overview

Syncbox is a versatile and efficient tool designed to synchronize files between a local filesystem and a remote storage system. Supporting various transfer protocols, Syncbox offers flexibility and performance in managing file synchronization tasks. With its intuitive command-line interface, Syncbox is both powerful for experienced users and accessible for beginners.

## Features

- **Support for Multiple Transfer Protocols**: Syncbox can synchronize files using FTP, local filesystems, and AWS S3.
- **Checksum Verification**: Files are verified based on checksums, ensuring integrity and consistency during synchronization.
- **Concurrent Uploads**: Leverage multi-threaded uploads for faster synchronization.
- **Dry Run Option**: Preview changes before they are made, enhancing control over file synchronization.
- **Checksum-Only Mode**: Generate and work with checksum files without performing actual synchronization.
- **Customizable File Size Threshold**: Define size limits to switch between metadata-based checksums and SHA256 digest.

## Installation

1. use Homebrew

```
brew tap romanschejbal/tap
brew install syncbox
```

2. or, ensure you have Rust and Cargo installed on your system. You can then install Syncbox by cloning the repository and building it using Cargo:

```bash
git clone https://github.com/romanschejbal/syncbox.git
cd syncbox
cargo build --release
```

## Usage

To use Syncbox, run the compiled binary with your desired options. The basic usage pattern is as follows:

```bash
syncbox [OPTIONS] transport [TRANSPORT OPTIONS]
```

### Options

- `--checksum_file`: Set the name of the checksum file. Default is `.syncbox.json.gz`.
- `--checksum_only`: Skip execution and only create the checksum file.
- `--dry_run`: Run without making any changes.
- `--force`: Ignore corrupted checksum files and override.
- `--concurrency`: Set the concurrency limit for file processing.
- `--file_size_threshold`: Set the threshold file size (in MB) for SHA256 digest vs. metadata check.
- `--skip_removal`: Skip the removal of files in the target directory.
- `--directory`: Specify the directory to synchronize.

### Transport Options

- **FTP**: Provide FTP host, user, password, directory, and TLS usage details.
- **Local**: Specify the local destination directory.
- **S3**: Set AWS S3 bucket details including region, access key, secret key, storage class, and directory.

For detailed command options and examples, run:

```bash
syncbox --help
```

## Contributing

Contributions to Syncbox are welcome! Please read our contributing guidelines to get started.

## License

Syncbox is licensed under [MIT License](LICENSE).
