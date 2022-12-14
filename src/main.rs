use clap::Parser;
use console::style;
use core::panic;
use futures::{stream, StreamExt};
use indicatif::ProgressStyle;
use std::{
    collections::HashMap,
    error::Error,
    ffi::OsString,
    io::Cursor,
    path::Path,
    sync::{Arc, Mutex},
};
use syncbox::{
    checksum_tree::ChecksumTree,
    reconciler::{Action, Reconciler},
    transport::{ftp::Ftp, Transport},
};
use tokio::fs;

const PROGRESS_BAR_CHARS: &str = "=ยป-";

/// Syncbox like dropbox, but with arbitrary tranfer protocol
#[derive(Parser, Debug)]
#[command(version, about = "Fast sync with remote filesystem")]
struct Args {
    #[arg(help = "Directory to diff against", default_value = ".")]
    directory: String,

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

    #[arg(
        long,
        help = "Dry run, won't make any changes",
        default_value_t = false
    )]
    dry_run: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();

    let args = Args::parse();
    let now = std::time::Instant::now();

    std::env::set_current_dir(args.directory)?;

    println!("{} ๐ Resolving files", style("[1/9]").dim().bold());

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
    println!("{} ๐งฌ Calculating checksums", style("[2/9]").dim().bold());
    let pb = &indicatif::ProgressBar::new(files.len().try_into()?);
    pb.set_style(
        ProgressStyle::with_template(
            "         [{elapsed_precise}] {bar:50.cyan/blue} {pos:>7}/{len:7} {wide_msg}",
        )
        .unwrap()
        .progress_chars(PROGRESS_BAR_CHARS),
    );
    let next_checksum_tree: ChecksumTree = stream::iter(files)
        .map(|filepath| async move {
            pb.set_message(filepath.clone());
            let checksum = sha256::digest_file(&filepath)
                .map_err(|e| format!("Failed checksum of {filepath:?} with error {e:?}"))?;
            pb.inc(1);
            Ok((filepath, checksum)) as Result<_, Box<dyn Error>>
        })
        .buffer_unordered(num_cpus::get())
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<HashMap<String, String>, _>>()?
        .into();
    pb.finish_and_clear();

    if args.checksum_only {
        println!("         Writing checksum file to {}", args.checksum_file);
        fs::write(
            Path::new(&args.checksum_file),
            serde_json::to_string_pretty(&next_checksum_tree)?,
        )
        .await?;
        return Ok(());
    }

    // get previous checksums using Transport
    println!(
        "{} ๐ Fetching last checksum file",
        style("[3/9]").dim().bold(),
    );

    let mut transport: Box<dyn Transport> = if args.dry_run {
        Box::new(syncbox::transport::local::LocalFilesystem::default())
    } else {
        Box::new(
            Ftp::new(args.ftp_host, args.ftp_user, args.ftp_pass, args.ftp_dir)
                .connect(args.use_tls)
                .await?,
        )
    };

    let previous_checksum_tree = transport
        .get_last_checksum(Path::new(&args.checksum_file))
        .await?;

    // reconcile
    println!("{} ๐ Reconciling changes", style("[4/9]").dim().bold(),);
    let todo = Reconciler::reconcile(previous_checksum_tree, &next_checksum_tree)?;

    if todo.is_empty() {
        println!("      ๐คท Nothing to do");
        return Ok(());
    }

    println!(
        "{} ๐ Executing {} action(s)",
        style("[5/9]").dim().bold(),
        style(todo.len()).bold()
    );
    let next_checksum_tree = Arc::new(Mutex::new(next_checksum_tree));

    let mut has_error = false;

    // first create directories
    println!("{} ๐ Creating directories", style("[6/9]").dim().bold(),);
    let create_directory_actions: Vec<_> = todo
        .iter()
        .filter(|action| matches!(action, Action::Mkdir(_)))
        .collect();
    for (i, action) in create_directory_actions.iter().enumerate() {
        let n = std::time::Instant::now();
        match action {
            Action::Mkdir(path) => match transport.mkdir(path.as_path()).await {
                Ok(_) => println!(
                    "      โ Creating directory {}/{} {:?} in {:.2?}s",
                    i + 1,
                    create_directory_actions.len(),
                    path,
                    n.elapsed().as_secs_f64(),
                ),
                Err(error) => {
                    eprintln!(
                        "      โ Error while creating directory {:?}: {}",
                        path, error
                    );
                    next_checksum_tree.lock().unwrap().remove_at(path);
                    has_error = true;
                }
            },
            _ => unreachable!(),
        };
    }

    // upload files
    let mut bytes = 0;
    println!("{} ๐ Uploading files", style("[7/9]").dim().bold(),);
    let put_actions: Vec<_> = todo
        .iter()
        .filter(|action| matches!(action, Action::Put(_)))
        .collect();
    for (i, action) in put_actions.iter().enumerate() {
        let n = std::time::Instant::now();

        match action {
            Action::Put(path) => {
                let file = fs::File::open(path).await?;
                let pb = Arc::new(indicatif::ProgressBar::new(file.metadata().await?.len()));
                pb.set_style(
                    ProgressStyle::with_template(
                        "         [{elapsed_precise}] {wide_bar:.cyan/blue} {bytes}/{total_bytes} [{bytes_per_sec}] {msg}",
                    )
                    .unwrap()
                    .progress_chars(PROGRESS_BAR_CHARS),
                );
                let msg = path.to_path_buf().to_str().unwrap().to_string();
                pb.set_message(msg);
                let pb_inner = Arc::clone(&pb);
                match transport
                    .write(
                        path,
                        Box::new(file),
                        Box::new(move |uploaded| {
                            pb_inner.inc(uploaded);
                        }),
                    )
                    .await
                {
                    Ok(b) => {
                        pb.finish_and_clear();
                        println!(
                            "      โ Copied {}/{} file: {:?} {} in {:.2?}s",
                            i + 1,
                            put_actions.len(),
                            path,
                            b.to_human_size(),
                            n.elapsed().as_secs_f64(),
                        );
                        bytes += b;
                    }
                    Err(error) => {
                        eprintln!("      โ Error while copying {:?}: {}", path, error);
                        next_checksum_tree.lock().unwrap().remove_at(path);
                        has_error = true;
                    }
                };
            }
            _ => unreachable!(),
        };
    }

    // removing files
    println!("{} ๐งป Removing files", style("[8/9]").dim().bold(),);
    let remove_actions: Vec<_> = todo
        .iter()
        .filter(|action| matches!(action, Action::Remove(_)))
        .collect();
    for (i, action) in remove_actions.iter().enumerate() {
        let n = std::time::Instant::now();

        match action {
            Action::Remove(path) => {
                match transport.remove(path).await {
                    Ok(_) => {
                        println!(
                            "      โ Removed {}/{} file: {:?} in {:.2?}s",
                            i + 1,
                            remove_actions.len(),
                            path,
                            n.elapsed().as_secs_f64(),
                        );
                    }
                    Err(error) => {
                        eprintln!("      โ Error while removing {:?}: {}", path, error);
                        has_error = true;
                    }
                };
            }
            _ => unreachable!(),
        };
    }

    println!("{} ๐ Uploading checksum", style("[9/9]").dim().bold(),);
    // save checksum
    let next_checksum_tree = Arc::try_unwrap(next_checksum_tree)
        .map_err(|_| <&str as Into<Box<dyn Error>>>::into("Failed to update checksums"))?
        .into_inner()?;
    let json = serde_json::to_string_pretty(&next_checksum_tree)?;

    transport
        .write(
            Path::new(&args.checksum_file),
            Box::new(Cursor::new(json.as_bytes().to_vec())),
            Box::new(|_| {}),
        )
        .await?;

    transport.close().await?;

    println!(
        "      โจ Done. Transfered {} in {:.2?}s",
        bytes.to_human_size(),
        now.elapsed().as_secs_f64()
    );

    if has_error {
        panic!("There were errors");
    }

    Ok(())
}

trait HumanBytes {
    fn to_human_size(self) -> String;
}

impl HumanBytes for u64 {
    fn to_human_size(self) -> String {
        if self > 1024 * 1024 {
            format!("{:.2?}MB", self as f64 / 1024.0 / 1024.0)
        } else if self > 1024 {
            format!("{:.2?}KB", self as f64 / 1024.0)
        } else {
            format!("{}B", self)
        }
    }
}
