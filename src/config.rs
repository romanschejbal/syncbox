use clap::{
    builder::{styling::AnsiColor, Styles},
    Parser,
};

fn get_styles() -> Styles {
    Styles::styled()
        .header(AnsiColor::Green.on_default())
        .usage(AnsiColor::Green.on_default())
        .literal(AnsiColor::Green.on_default())
        .placeholder(AnsiColor::Green.on_default())
}

/// Fast sync with remote filesystem
#[derive(Parser, Debug, Clone)]
#[command(version, about, styles = get_styles())]
pub struct Args {
    #[arg(
        long,
        help = "Name of the checksum file",
        default_value = "./.syncbox.json.gz",
        env = "SYNCBOX_CHECKSUM_FILE"
    )]
    pub checksum_file: String,

    #[arg(
        long,
        help = "Will skip execution and only creates the checksum file",
        default_value_t = false
    )]
    pub checksum_only: bool,

    #[arg(
        short,
        long,
        help = "Will upload checksum file every N files",
        default_value_t = 0,
        env = "SYNCBOX_INTERMITTENT_CHECKSUM_UPLOAD"
    )]
    pub intermittent_checksum_upload: usize,

    #[command(subcommand)]
    pub transport: TransportType,

    #[arg(
        long,
        help = "Ignore corrupted checksum file and override",
        default_value_t = false
    )]
    pub force: bool,

    #[arg(
        short,
        long,
        help = "Concurrency limit for file operations",
        default_value_t = 1,
        env = "SYNCBOX_CONCURRENCY"
    )]
    pub concurrency: usize,

    #[arg(
        long,
        help = "Files of size below this threshold (in MBs) will be read and digested using SHA256, the others will use metadata as the checksum",
        default_value_t = 100,
        env = "SYNCBOX_FILE_THRESHOLD"
    )]
    pub file_size_threshold: u64,

    #[arg(short, long, default_value_t = false)]
    pub skip_removal: bool,

    #[arg(
        help = "Directory to diff against",
        default_value = ".",
        env = "SYNCBOX_DIRECTORY"
    )]
    pub directory: String,

    #[arg(long, help = "Skip first X actions", default_value_t = 0)]
    pub skip: usize,

    #[arg(
        long,
        help = "Maximum number of retry attempts for failed operations",
        default_value_t = 3,
        env = "SYNCBOX_MAX_RETRIES"
    )]
    pub max_retries: usize,

    #[arg(
        long,
        help = "Initial retry delay in milliseconds",
        default_value_t = 500,
        env = "SYNCBOX_INITIAL_RETRY_DELAY"
    )]
    pub initial_retry_delay: u64,

    #[arg(
        long,
        help = "Maximum retry delay in seconds",
        default_value_t = 30,
        env = "SYNCBOX_MAX_RETRY_DELAY"
    )]
    pub max_retry_delay: u64,

    #[arg(
        long,
        help = "Enable automatic retry for transport operations",
        default_value_t = false,
        env = "SYNCBOX_ENABLE_RETRY_TRANSPORT"
    )]
    pub enable_retry_transport: bool,
}

#[derive(Clone, Debug, Parser)]
pub enum TransportType {
    Ftp {
        #[arg(long, env = "FTP_HOST")]
        ftp_host: String,
        #[arg(long, env = "FTP_USER")]
        ftp_user: String,
        #[arg(long, env = "FTP_PASS")]
        ftp_pass: String,
        #[arg(long, default_value = ".", env = "FTP_DIR")]
        ftp_dir: String,
        #[arg(long, default_value_t = false, env = "FTP_USE_TLS")]
        use_tls: bool,
    },
    Sftp {
        #[arg(long, env = "SFTP_HOST")]
        host: String,
        #[arg(long, env = "SFTP_USER")]
        user: String,
        #[arg(long, env = "SFTP_PASS")]
        pass: String,
        #[arg(long, default_value = ".", env = "SFTP_DIR")]
        dir: String,
    },
    Local {
        #[arg(long, short)]
        destination: String,
    },
    S3 {
        #[arg(long, env = "S3_BUCKET")]
        bucket: String,
        #[arg(long, env = "S3_REGION")]
        region: String,
        #[arg(long, env = "S3_ACCESS_KEY")]
        access_key: String,
        #[arg(long, env = "S3_SECRET_KEY")]
        secret_key: String,
        #[arg(long, default_value = "STANDARD", env = "S3_STORAGE_CLASS")]
        storage_class: String,
        #[arg(long, default_value = ".", env = "S3_DIRECTORY")]
        directory: String,
    },
    Dry,
}
