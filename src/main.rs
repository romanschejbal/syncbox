use clap::Parser;
use console::style;
use core::panic;
use futures::{future::try_join_all, stream, StreamExt};
use indicatif::ProgressStyle;
use std::{
    collections::HashMap,
    error::Error,
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64},
        Arc,
    },
    time::SystemTime,
};
use syncbox::{
    checksum_tree::ChecksumTree,
    reconciler::{Action, Reconciler},
    transport::{ftp::Ftp, local::LocalFilesystem, s3::AwsS3, Transport},
};
use tokio::{fs, sync::Mutex};

const PROGRESS_BAR_CHARS: &str = "▰▰▱";
const DEFAULT_FILE_SIZE_THRESHOLD: u64 = 1;

/// Syncbox like dropbox, but with arbitrary tranfer protocol
#[derive(Parser, Debug, Clone)]
#[command(version, about = "Fast sync with remote filesystem")]
struct Args {
    #[arg(
        long,
        help = "Name of the checksum file",
        default_value = ".syncbox.json"
    )]
    checksum_file: String,

    #[arg(
        long,
        help = "Will skip execution and only creates the checksum file",
        default_value_t = false
    )]
    checksum_only: bool,

    #[command(subcommand)]
    transport: TransportType,

    #[arg(
        long,
        help = "Dry run, won't make any changes",
        default_value_t = false
    )]
    dry_run: bool,

    #[arg(
        long,
        help = "Ignore corrupted checksum file and override",
        default_value_t = false
    )]
    force: bool,

    #[arg(long, help = "Concurrency limit", default_value_t = 1)]
    concurrency: usize,

    #[arg(long, help = "Files of size below this threshold (in MBs) will be read and digested using SHA256, the others will use metadata as the checksum", default_value_t = DEFAULT_FILE_SIZE_THRESHOLD)]
    file_size_threshold: u64,

    #[arg(long, default_value_t = false)]
    skip_removal: bool,

    #[arg(help = "Directory to diff against", default_value = ".")]
    directory: String,
}

#[derive(Clone, Debug, Parser)]
enum TransportType {
    Ftp {
        #[arg(long)]
        ftp_host: String,
        #[arg(long)]
        ftp_user: String,
        #[arg(long)]
        ftp_pass: String,
        #[arg(long)]
        ftp_dir: String,
        #[arg(long, default_value_t = false)]
        use_tls: bool,
    },
    Local {
        #[arg(long)]
        destination: String,
    },
    S3 {
        #[arg(long)]
        bucket: String,
        #[arg(long)]
        region: String,
        #[arg(long)]
        access_key: String,
        #[arg(long)]
        secret_key: String,
        #[arg(long, default_value = "STANDARD")]
        storage_class: String,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
    env_logger::init();

    let args = Args::parse();
    let now = std::time::Instant::now();

    std::env::set_current_dir(args.directory.clone())?;

    println!("{} 🔍 Resolving files", style("[1/9]").dim().bold());

    let mut ignored_files = vec![OsString::from(".git"), OsString::from(".syncboxignore")];
    ignored_files.push((&args.checksum_file).into());
    let walker = ignore::WalkBuilder::new("./")
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
            "         [{elapsed_precise}] {bar:50.cyan/blue} {pos:>7}/{len:7} {wide_msg}",
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
        println!("      💿 Writing checksum file to {}", args.checksum_file);
        fs::write(
            Path::new(&args.checksum_file),
            serde_json::to_string_pretty(&next_checksum_tree)?,
        )
        .await?;
        return Ok(());
    }

    // get previous checksums using Transport
    println!(
        "{} 📄 Fetching last checksum file",
        style("[3/9]").dim().bold(),
    );

    let mut transport = make_transport(&args).await?;

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
    let todo = Reconciler::reconcile(previous_checksum_tree, &next_checksum_tree)?;

    if todo.is_empty() {
        println!("      🤷 Nothing to do");
        return Ok(());
    }

    println!(
        "{} 🚀 Executing {} action(s)",
        style("[5/9]").dim().bold(),
        style(todo.len()).bold()
    );

    if args.dry_run {
        println!("      🚨 Dry run, no changes will be made");
        return Ok(());
    }

    let has_error = &AtomicBool::new(false);

    // first create directories
    println!("{} 📂 Creating directories", style("[6/9]").dim().bold());
    let create_directory_actions: Vec<_> = todo
        .iter()
        .filter(|action| matches!(action, Action::Mkdir(_)))
        .collect();
    for (i, action) in create_directory_actions.iter().enumerate() {
        let n = std::time::Instant::now();
        match action {
            Action::Mkdir(path) => match transport.mkdir(path.as_path()).await {
                Ok(_) => println!(
                    "      ✅ Creating directory {}/{} {:?} in {:.2?}s",
                    i + 1,
                    create_directory_actions.len(),
                    path,
                    n.elapsed().as_secs_f64(),
                ),
                Err(error) => {
                    eprintln!(
                        "      ❌ Error while creating directory {}/{} {:?}: {}",
                        i + 1,
                        create_directory_actions.len(),
                        path,
                        error
                    );
                    has_error.store(true, std::sync::atomic::Ordering::SeqCst);
                }
            },
            _ => unreachable!(),
        };
    }

    // upload files
    println!("{} 🏂 Uploading files", style("[7/9]").dim().bold());
    let bytes = &AtomicU64::new(0);
    let progress_bars = &indicatif::MultiProgress::new();
    let next_checksum_tree = &Mutex::new(next_checksum_tree);
    let transports =
        &Mutex::new(try_join_all((0..args.concurrency).map(|_| make_transport(&args))).await?);
    let put_actions = todo
        .iter()
        .filter(|action| matches!(action, Action::Put(_)))
        .collect::<Vec<_>>();
    let put_actions_len = put_actions.len();
    let put_actions = put_actions.iter()
        .enumerate()
        .map(|(i, action)| async move {

            let n = std::time::Instant::now();

            let Action::Put(path) = action else {
                unreachable!();
            };

            let mut transport = transports.lock().await.pop().unwrap();
            let file = fs::File::open(&path).await.unwrap();
            let metadata = fs::metadata(&path).await.unwrap();
            let pb = indicatif::ProgressBar::new(file.metadata().await.unwrap().len());
            let pb = Arc::new(progress_bars.add(pb));
            let mut template = format!("         [{}/{}] ", i + 1, put_actions_len);
            template.push_str("[{elapsed_precise}] {wide_bar:.cyan/black} {bytes}/{total_bytes} [{bytes_per_sec}] {msg}");
            pb.set_style(
                ProgressStyle::with_template(&template)
                .unwrap()
                .progress_chars(PROGRESS_BAR_CHARS),
            );
            let msg = path.to_path_buf().to_str().unwrap().to_string();
            pb.set_message(msg);
            pb.inc(0);
            let pb_inner = Arc::clone(&pb);
            match transport
                .write(
                    path,
                    Box::new(file),
                    Box::new(move |uploaded| {
                        pb_inner.inc(uploaded);
                    }),
                    metadata.len()
                )
                .await
            {
                Ok(b) => {
                    pb.finish_and_clear();
                    progress_bars.remove(&pb);
                    if args.concurrency == 1 {
                        println!(
                            "      ✅ Copied {}/{} file: {:?} {} in {:.2?}s",
                            i + 1,
                            put_actions_len,
                            path,
                            b.to_human_size(),
                            n.elapsed().as_secs_f64(),
                        );
                    }
                    bytes.fetch_add(b, std::sync::atomic::Ordering::SeqCst);
                }
                Err(error) => {
                    pb.finish_and_clear();
                    progress_bars.remove(&pb);
                    eprintln!("      ❌ Error while copying {:?}: {}", path, error);
                    next_checksum_tree.lock().await.remove_at(path);
                    has_error.store(true, std::sync::atomic::Ordering::SeqCst);
                }
            };
            transports.lock().await.push(transport);
        });

    stream::iter(put_actions)
        .buffer_unordered(args.concurrency)
        .collect::<Vec<_>>()
        .await;

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
            .collect();
        let remove_actions_len = remove_actions.len();
        let remove_actions = remove_actions
            .iter()
            .enumerate()
            .map(|(i, action)| async move {
                let mut transport = transports.lock().await.pop().unwrap();

                let n = std::time::Instant::now();

                match action {
                    Action::Remove(path) => {
                        match transport.remove(path).await {
                            Ok(_) => {
                                println!(
                                    "      ✅ Removed {}/{} file: {:?} in {:.2?}s",
                                    i + 1,
                                    remove_actions_len,
                                    path,
                                    n.elapsed().as_secs_f64(),
                                );
                            }
                            Err(error) => {
                                eprintln!("      ❌ Error while removing {:?}: {}", path, error);
                                has_error.store(true, std::sync::atomic::Ordering::SeqCst);
                            }
                        };
                    }
                    _ => unreachable!(),
                };
                transports.lock().await.push(transport);
            });

        stream::iter(remove_actions)
            .buffer_unordered(args.concurrency)
            .collect::<Vec<_>>()
            .await;
    }

    let mut transport = make_transport(&args).await?;

    println!("{} 🏁 Uploading checksum", style("[9/9]").dim().bold());
    transport
        .write_last_checksum(
            Path::new(&args.checksum_file),
            &*next_checksum_tree.lock().await,
            Box::new(|_| {}),
        )
        .await?;

    transport.close().await?;

    println!(
        "      ✨ Done. Transfered {} in {:.2?}s",
        bytes.to_human_size(),
        now.elapsed().as_secs_f64()
    );

    if has_error.load(std::sync::atomic::Ordering::SeqCst) {
        panic!("There were errors");
    }

    Ok(())
}

async fn make_transport(
    args: &Args,
) -> Result<Box<dyn Transport>, Box<dyn Error + Send + Sync + 'static>> {
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
        TransportType::Local { destination } => Box::new(LocalFilesystem::new(destination)),
        TransportType::S3 {
            bucket,
            region,
            access_key,
            secret_key,
            storage_class,
        } => Box::new(AwsS3::new(
            bucket,
            region,
            access_key,
            secret_key,
            storage_class,
        )?),
    })
}

trait HumanBytes {
    fn to_human_size(self) -> String;
}

impl HumanBytes for u64 {
    fn to_human_size(self) -> String {
        let value = self;
        if value > 1024 * 1024 {
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
        let value = self.load(std::sync::atomic::Ordering::SeqCst);
        value.to_human_size()
    }
}
