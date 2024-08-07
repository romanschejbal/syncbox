use clap::{
    builder::{styling::AnsiColor, Styles},
    Parser,
};
use console::style;
use core::panic;
use futures::{future::try_join_all, stream, StreamExt};
use indicatif::ProgressStyle;
use std::{
    collections::{HashMap, HashSet},
    error::Error,
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering::SeqCst},
        Arc,
    },
    time::SystemTime,
};
use syncbox::{
    checksum_tree::ChecksumTree,
    progress,
    reconciler::{Action, Reconciler},
    transport::{
        dry::DryTransport, ftp::Ftp, local::LocalFilesystem, s3::AwsS3, sftp::SFtp, Transport,
    },
};
use tokio::{fs, sync::Mutex};

const PROGRESS_BAR_CHARS: &str = "▰▰▱";
const DEFAULT_FILE_SIZE_THRESHOLD: u64 = 1;

fn get_styles() -> Styles {
    Styles::styled()
        .header(AnsiColor::Yellow.on_default())
        .usage(AnsiColor::Green.on_default())
        .literal(AnsiColor::Green.on_default())
        .placeholder(AnsiColor::Green.on_default())
}

/// Fast sync with remote filesystem
#[derive(Parser, Debug, Clone)]
#[command(version, about, styles = get_styles())]
struct Args {
    #[arg(
        long,
        help = "Name of the checksum file",
        default_value = "./.syncbox.json.gz",
        env = "SYNCBOX_CHECKSUM_FILE"
    )]
    checksum_file: String,

    #[arg(
        long,
        help = "Will skip execution and only creates the checksum file",
        default_value_t = false
    )]
    checksum_only: bool,

    #[arg(
        short,
        long,
        help = "Will upload checksum file every N files",
        default_value_t = 0,
        env = "SYNCBOX_INTERMITTENT_CHECKSUM_UPLOAD"
    )]
    intermittent_checksum_upload: usize,

    #[command(subcommand)]
    transport: TransportType,

    #[arg(
        long,
        help = "Ignore corrupted checksum file and override",
        default_value_t = false
    )]
    force: bool,

    #[arg(
        short,
        long,
        help = "Concurrency limit",
        default_value_t = 1,
        env = "SYNCBOX_CONCURRENCY"
    )]
    concurrency: usize,

    #[arg(
        long,
        help = "Files of size below this threshold (in MBs) will be read and digested using SHA256, the others will use metadata as the checksum",
        default_value_t = DEFAULT_FILE_SIZE_THRESHOLD,
        env = "SYNCBOX_FILE_THRESHOLD"
    )]
    file_size_threshold: u64,

    #[arg(short, long, default_value_t = false)]
    skip_removal: bool,

    #[arg(
        help = "Directory to diff against",
        default_value = ".",
        env = "SYNCBOX_DIRECTORY"
    )]
    directory: String,

    #[arg(long, help = "Skip first X actions", default_value_t = 0)]
    skip: usize,
}

#[derive(Clone, Debug, Parser)]
enum TransportType {
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
    dotenvy::from_filename(".env.syncbox").ok();
    dotenvy::dotenv().ok();

    let args = Args::parse();
    let now = std::time::Instant::now();

    std::env::set_current_dir(args.directory.clone())?;

    println!("{} 🔍 Resolving files", style("[1/9]").dim().bold());

    let mut ignored_files = vec![
        OsString::from(".git"),
        OsString::from(".syncboxignore"),
        OsString::from(".DS_Store"),
    ];
    ignored_files.push((&args.checksum_file).into());
    let walker = ignore::WalkBuilder::new(".")
        .hidden(false)
        .filter_entry(move |entry| !ignored_files.contains(&entry.file_name().to_os_string()))
        .add_custom_ignore_filename(".syncboxignore")
        .build();
    let files = walker
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|entry| entry.file_type().map_or(false, |t| t.is_file()))
        .map(|entry| entry.path().to_string_lossy().to_string())
        .collect::<Vec<_>>();

    // build map with checksums
    println!("{} 🧬 Calculating checksums", style("[2/9]").dim().bold());
    let pb = &indicatif::ProgressBar::new(files.len().try_into()?);
    pb.set_style(
        ProgressStyle::with_template(
            "[{elapsed_precise}] {bar:50.cyan/blue} {pos:>7}/{len:7} {wide_msg}",
        )
        .unwrap()
        .progress_chars(PROGRESS_BAR_CHARS),
    );
    let next_checksum_tree: ChecksumTree = stream::iter(files)
        .map(|filepath| {
            let pb = pb.clone();
            tokio::spawn(async move {
                pb.set_message(filepath.clone());
                let path_buf = PathBuf::from(filepath.clone());
                let metadata = tokio::fs::metadata(path_buf.as_path()).await.unwrap();
                let checksum = if metadata.len() > args.file_size_threshold * 1024 * 1024 {
                    format!(
                        "s{}_c{}_m{}",
                        metadata.len(),
                        metadata
                            .created()?
                            .duration_since(SystemTime::UNIX_EPOCH)?
                            .as_secs(),
                        metadata
                            .modified()?
                            .duration_since(SystemTime::UNIX_EPOCH)?
                            .as_secs()
                    )
                } else {
                    sha256::try_digest(path_buf.as_path())
                        .map_err(|e| format!("Failed checksum of {filepath:?} with error {e:?}"))?
                };
                pb.inc(1);
                Ok((filepath, checksum)) as Result<_, Box<dyn Error + Send + Sync + 'static>>
            })
        })
        .buffer_unordered(num_cpus::get())
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .collect::<Result<HashMap<String, String>, _>>()?
        .into();
    pb.finish_and_clear();

    if args.checksum_only {
        println!("💿 Writing checksum file to {}", args.checksum_file);
        fs::write(
            Path::new(&args.checksum_file),
            next_checksum_tree.to_gzip()?,
        )
        .await?;
        return Ok(());
    }

    // get previous checksums using Transport
    println!(
        "{} 📄 Fetching last checksum file",
        style("[3/9]").dim().bold(),
    );

    let mut transport = make_transport(&args)
        .await
        .map_err(|e| format!("Connection failed with error: {e}"))?;

    let previous_checksum_tree = match transport
        .read_last_checksum(Path::new(&args.checksum_file))
        .await
    {
        Ok(checksum) => checksum,
        Err(e) => {
            if args.force {
                ChecksumTree::default()
            } else {
                panic!("{e}");
            }
        }
    };

    // reconcile
    println!("{} 🚚 Reconciling changes", style("[4/9]").dim().bold(),);
    let todo = Arc::new(Reconciler::reconcile(
        previous_checksum_tree,
        &next_checksum_tree,
    )?);

    if todo.is_empty() {
        println!("      🤷 Nothing to do");
        return Ok(());
    }

    println!(
        "{} 🚀 Executing {} action(s)",
        style("[5/9]").dim().bold(),
        style(todo.len()).bold()
    );

    let has_error = Arc::new(AtomicBool::new(false));

    // first create directories
    println!("{} 📂 Creating directories", style("[6/9]").dim().bold());
    let create_directory_actions: Vec<_> = todo
        .iter()
        .filter(|action| matches!(action, Action::Mkdir(_)))
        .collect();
    for (i, action) in create_directory_actions.iter().enumerate() {
        if i < args.skip {
            continue;
        }

        let n = std::time::Instant::now();
        match action {
            Action::Mkdir(path) => match transport.mkdir(path.as_path()).await {
                Ok(_) => println!(
                    "✅ Creating directory {}/{} {:?} in {:.2?}s",
                    i + 1,
                    create_directory_actions.len(),
                    path,
                    n.elapsed().as_secs_f64(),
                ),
                Err(error) => {
                    eprintln!(
                        "❌ Error while creating directory {}/{} {:?}: {}",
                        i + 1,
                        create_directory_actions.len(),
                        path,
                        error
                    );
                    has_error.store(true, SeqCst);
                }
            },
            _ => unreachable!(),
        };
    }

    let checksum_path = Arc::new(PathBuf::from(&args.checksum_file));

    // upload files
    let bytes = Arc::new(AtomicU64::new(0));
    let progress_bars = Arc::new(indicatif::MultiProgress::new());
    let next_checksum_tree = Arc::new(Mutex::new(next_checksum_tree));
    let transports = Arc::new(Mutex::new(
        try_join_all((0..args.concurrency).map(|_| make_transport(&args))).await?,
    ));
    let mut put_actions = todo
        .iter()
        .filter(|action| matches!(action, Action::Put(_)))
        .cloned()
        .collect::<Vec<_>>();
    put_actions.sort_by(|a, b| {
        let Action::Put(a) = a else { unreachable!() };
        let Action::Put(b) = b else { unreachable!() };
        if std::fs::metadata(a).unwrap().len() < std::fs::metadata(b).unwrap().len() {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Greater
        }
    });
    let put_actions = Arc::new(put_actions);
    let total_to_upload = Arc::new(AtomicU64::new(
        put_actions
            .iter()
            .map(|action| {
                let Action::Put(path) = action else {
                    unreachable!();
                };
                std::fs::metadata(path).unwrap().len()
            })
            .sum::<u64>(),
    ));
    println!(
        "{} 🏂 Uploading {} files ({})",
        style("[7/9]").dim().bold(),
        put_actions.len(),
        total_to_upload.to_human_size()
    );
    let put_actions_len = put_actions.len();
    let finished_paths = Arc::new(Mutex::new(HashSet::new()));
    let put_actions = put_actions.iter()
        .enumerate()
        .skip((args.skip as i64 - create_directory_actions.len() as i64).max(0) as usize)
        .map(|(i, action)| {
            let total_to_upload = Arc::clone(&total_to_upload);
            let checksum_path = Arc::clone(&checksum_path);
            let todo = Arc::clone(&todo);
            let finished_paths = Arc::clone(&finished_paths);
            let transports = Arc::clone(&transports);
            let progress_bars = Arc::clone(&progress_bars);
            let bytes = Arc::clone(&bytes);
            let next_checksum_tree = Arc::clone(&next_checksum_tree);
            let has_error = Arc::clone(&has_error);
            let action = action.clone();
            tokio::spawn(async move {
                let Action::Put(path) = action else {
                    unreachable!();
                };

                let file = fs::File::open(&path).await.unwrap();
                let metadata = file.metadata().await.unwrap();
                let mut transport = transports.lock().await.pop().unwrap();
                let pb = indicatif::ProgressBar::new(metadata.len());
                let pb = Arc::new(progress_bars.add(pb));
                let mut template = format!("[{}/{}] ", i + 1, put_actions_len);
                template.push_str("[{elapsed_precise}] {wide_bar:.cyan/blue} {bytes}/{total_bytes} [{bytes_per_sec}] {msg}");
                pb.set_style(
                    ProgressStyle::with_template(&template)
                    .unwrap()
                    .progress_chars(PROGRESS_BAR_CHARS),
                );
                let msg = path.to_path_buf().to_str().unwrap().to_string();
                pb.set_message(msg);
                pb.inc(0);
                let pb_inner = Arc::clone(&pb);
                let file = progress::ProgressStream::new(file,Box::new(move |uploaded| {
                    pb_inner.set_position(uploaded);
                }));
                match transport
                    .write(
                        path.as_path(),
                        Box::new(file),
                        metadata.len()
                    )
                    .await
                {
                    Ok(b) => {
                        bytes.fetch_add(b, SeqCst);
                        finished_paths.lock().await.insert(path.clone());
                        let message = format!("{} | {} remaining",
                            path.to_string_lossy(),
                            (total_to_upload.load(SeqCst) - bytes.load(SeqCst)).to_human_size(),
                        );
                        pb.finish_with_message(message.clone());

                        // if we are running on the CI, print successful message
                        if std::env::var("CI").is_ok() {
                            println!("✅ {}", message);
                        }

                        // if we are uploading checksums intermittently, do it now
                        if args.intermittent_checksum_upload > 0
                            && finished_paths.lock().await.len() > 0 && finished_paths.lock().await.len()
                                % args.intermittent_checksum_upload
                                == 0
                        {
                            let mut intermittent_checksum = next_checksum_tree.lock().await.clone();
                            let finished_paths = finished_paths.lock().await;
                            todo.iter().filter_map(|action| {
                                let path = match action {
                                    Action::Put(path) => path,
                                    Action::Remove(path) => path,
                                    Action::Mkdir(_) => return None, // done already above
                                };
                                if !finished_paths.contains(path) {
                                    Some(path)
                                } else {
                                    None
                                }
                            }).for_each(|path| {
                                intermittent_checksum.remove_at(path);
                            });
                            pb.set_message("📸 Uploading intermittent checksum");
                            if let Err(e) = transport.write_last_checksum(checksum_path.as_path(), &intermittent_checksum).await {
                                pb.set_message(format!("❌ Error while uploading intermittent checksum: {}", e));
                            } else {
                                pb.set_message(message);
                            }
                        }
                    }
                    Err(error) => {
                        let message = format!("❌ Error while copying {:?}: {}", path, error);
                        pb.abandon_with_message(message.clone());
                        next_checksum_tree.lock().await.remove_at(path.as_path());
                        has_error.store(true, SeqCst);

                        // if we are running on the CI, print error message
                        if std::env::var("CI").is_ok() {
                            println!("{message}");
                        }
                    }
                };
                transports.lock().await.push(transport);
            })
        });

    stream::iter(put_actions)
        .buffer_unordered(args.concurrency)
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;

    // removing files
    if args.skip_removal {
        println!(
            "{} 🧻 Removing files (skipping)",
            style("[8/9]").dim().bold()
        );
    } else {
        println!("{} 🧻 Removing files", style("[8/9]").dim().bold());
        let remove_actions: Vec<_> = todo
            .iter()
            .filter(|action| matches!(action, Action::Remove(_)))
            .cloned()
            .collect();
        let remove_actions_len = remove_actions.len();
        let remove_actions = remove_actions
            .iter()
            .enumerate()
            .skip(
                (args.skip as i64 - create_directory_actions.len() as i64 - put_actions_len as i64)
                    .max(0) as usize,
            )
            .map(|(i, action)| {
                let transports = Arc::clone(&transports);
                let has_error = Arc::clone(&has_error);
                let action = action.clone();
                tokio::spawn(async move {
                    let mut transport = transports.lock().await.pop().unwrap();

                    let n = std::time::Instant::now();

                    match action {
                        Action::Remove(path) => {
                            match transport.remove(path.as_path()).await {
                                Ok(_) => {
                                    println!(
                                        "✅ Removed {}/{} file: {:?} in {:.2?}s",
                                        i + 1,
                                        remove_actions_len,
                                        path,
                                        n.elapsed().as_secs_f64(),
                                    );
                                }
                                Err(error) => {
                                    eprintln!("❌ Error while removing {:?}: {}", path, error);
                                    has_error.store(true, SeqCst);
                                }
                            };
                        }
                        _ => unreachable!(),
                    };
                    transports.lock().await.push(transport);
                })
            });

        stream::iter(remove_actions)
            .buffer_unordered(args.concurrency)
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?;
    }

    let mut transport = make_transport(&args).await?;

    println!("{} 🏁 Uploading checksum", style("[9/9]").dim().bold());
    transport
        .write_last_checksum(checksum_path.as_path(), &*next_checksum_tree.lock().await)
        .await?;

    transport.close().await?;

    println!(
        "✨ Done. Transfered {} in {:.2?}s",
        bytes.to_human_size(),
        now.elapsed().as_secs_f64()
    );

    if has_error.load(SeqCst) {
        panic!("There were errors");
    }

    Ok(())
}

async fn make_transport(
    args: &Args,
) -> Result<Box<dyn Transport + Send + Sync>, Box<dyn Error + Send + Sync + 'static>> {
    Ok(match &args.transport {
        TransportType::Ftp {
            ftp_host,
            ftp_user,
            ftp_pass,
            ftp_dir,
            use_tls,
        } => Box::new(
            Ftp::new(ftp_host, ftp_user, ftp_pass, ftp_dir)
                .connect(*use_tls)
                .await?,
        ),
        TransportType::Sftp {
            host,
            user,
            pass,
            dir,
        } => Box::new(SFtp::new(host, user, pass, dir).await?),
        TransportType::Local { destination } => Box::new(LocalFilesystem::new(destination)),
        TransportType::S3 {
            bucket,
            region,
            access_key,
            secret_key,
            storage_class,
            directory,
        } => Box::new(AwsS3::new(
            bucket,
            region,
            access_key,
            secret_key,
            storage_class,
            directory.into(),
        )?),
        TransportType::Dry => Box::new(DryTransport),
    })
}

trait HumanBytes {
    fn to_human_size(self) -> String;
}

impl HumanBytes for u64 {
    fn to_human_size(self) -> String {
        let value = self;
        if value > 1024 * 1024 * 1024 {
            format!("{:.2?}GB", value as f64 / 1024.0 / 1024.0 / 1024.0)
        } else if value > 1024 * 1024 {
            format!("{:.2?}MB", value as f64 / 1024.0 / 1024.0)
        } else if value > 1024 {
            format!("{:.2?}KB", value as f64 / 1024.0)
        } else {
            format!("{}B", value)
        }
    }
}

impl HumanBytes for &AtomicU64 {
    fn to_human_size(self) -> String {
        let value = self.load(SeqCst);
        value.to_human_size()
    }
}
