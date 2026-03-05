use crate::checksum_tree::ChecksumTree;
use crate::config::Args;
use crate::progress;
use crate::reconciler::{Action, Reconciler};
use crate::transport::{
    retry::{ConfigBasedTransportFactory, RetryConfig, RetryTransport, TransportFactory},
    Transport,
};
use crate::utils::HumanBytes;
use console::style;
use futures::{stream, StreamExt};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
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
use tokio::{fs, sync::Mutex};

const PROGRESS_BAR_CHARS: &str = "▰▰▱";

#[derive(Debug)]
pub struct SyncResult {
    pub files_uploaded: usize,
    pub bytes_transferred: u64,
    pub files_removed: usize,
    pub directories_created: usize,
    pub duration: std::time::Duration,
    pub had_errors: bool,
}

pub struct SyncEngine {
    args: Args,
}

impl SyncEngine {
    pub fn new(args: Args) -> Self {
        Self { args }
    }

    pub async fn sync(&self) -> Result<SyncResult, Box<dyn Error + Send + Sync + 'static>> {
        let start_time = std::time::Instant::now();

        // Phase 1: Discover files and calculate checksums
        let next_checksum_tree = self.discover_and_calculate_checksums().await?;

        if self.args.checksum_only {
            return self
                .save_checksum_only(&next_checksum_tree, start_time)
                .await;
        }

        // Phase 2: Fetch previous checksums
        let previous_checksum_tree = self.fetch_previous_checksums().await?;

        // Phase 3: Reconcile changes
        let actions = self.reconcile_changes(previous_checksum_tree, &next_checksum_tree)?;

        if actions.is_empty() {
            println!("      🤷 Nothing to do");
            return Ok(SyncResult {
                files_uploaded: 0,
                bytes_transferred: 0,
                files_removed: 0,
                directories_created: 0,
                duration: start_time.elapsed(),
                had_errors: false,
            });
        }

        // Phase 4: Execute operations
        let sync_stats = self
            .execute_sync_operations(actions, next_checksum_tree)
            .await?;

        Ok(SyncResult {
            files_uploaded: sync_stats.files_uploaded,
            bytes_transferred: sync_stats.bytes_transferred,
            files_removed: sync_stats.files_removed,
            directories_created: sync_stats.directories_created,
            duration: start_time.elapsed(),
            had_errors: sync_stats.had_errors,
        })
    }

    async fn discover_and_calculate_checksums(
        &self,
    ) -> Result<ChecksumTree, Box<dyn Error + Send + Sync + 'static>> {
        println!("{} 🔍 Resolving files", style("[1/9]").dim().bold());

        let mut ignored_files = vec![
            OsString::from(".git"),
            OsString::from(".syncboxignore"),
            OsString::from(".DS_Store"),
        ];
        ignored_files.push((&self.args.checksum_file).into());

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

        println!("{} 🧬 Calculating checksums", style("[2/9]").dim().bold());
        let pb = ProgressBar::new(files.len().try_into()?);
        pb.set_style(
            ProgressStyle::with_template(
                "[{elapsed_precise}] {bar:50.cyan/blue} {pos:>7}/{len:7} {wide_msg}",
            )
            .unwrap()
            .progress_chars(PROGRESS_BAR_CHARS),
        );

        let checksum_tree: ChecksumTree = stream::iter(files)
            .map(|filepath| {
                let pb = pb.clone();
                let file_size_threshold = self.args.file_size_threshold;
                tokio::spawn(async move {
                    pb.set_message(filepath.clone());
                    let result =
                        Self::calculate_file_checksum(&filepath, file_size_threshold).await;
                    pb.inc(1);
                    result
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
        Ok(checksum_tree)
    }

    async fn calculate_file_checksum(
        filepath: &str,
        file_size_threshold: u64,
    ) -> Result<(String, String), Box<dyn Error + Send + Sync + 'static>> {
        let path_buf = PathBuf::from(filepath);
        let metadata = tokio::fs::metadata(&path_buf).await?;

        let checksum = if metadata.len() > file_size_threshold * 1024 * 1024 {
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
            sha256::try_digest(&path_buf)
                .map_err(|e| format!("Failed checksum of {filepath:?} with error {e:?}"))?
        };

        Ok((filepath.to_string(), checksum))
    }

    async fn save_checksum_only(
        &self,
        checksum_tree: &ChecksumTree,
        start_time: std::time::Instant,
    ) -> Result<SyncResult, Box<dyn Error + Send + Sync + 'static>> {
        println!("💿 Writing checksum file to {}", self.args.checksum_file);
        fs::write(
            Path::new(&self.args.checksum_file),
            checksum_tree.to_gzip()?,
        )
        .await?;

        Ok(SyncResult {
            files_uploaded: 0,
            bytes_transferred: 0,
            files_removed: 0,
            directories_created: 0,
            duration: start_time.elapsed(),
            had_errors: false,
        })
    }

    async fn fetch_previous_checksums(
        &self,
    ) -> Result<ChecksumTree, Box<dyn Error + Send + Sync + 'static>> {
        println!(
            "{} 📄 Fetching last checksum file",
            style("[3/9]").dim().bold(),
        );

        let mut transport = self.create_transport().await?;

        match transport
            .read_last_checksum(Path::new(&self.args.checksum_file))
            .await
        {
            Ok(checksum) => Ok(checksum),
            Err(e) => {
                if self.args.force {
                    Ok(ChecksumTree::default())
                } else {
                    Err(format!("Failed to fetch previous checksums: {e}").into())
                }
            }
        }
    }

    fn reconcile_changes(
        &self,
        previous: ChecksumTree,
        current: &ChecksumTree,
    ) -> Result<Vec<Action>, Box<dyn Error + Send + Sync + 'static>> {
        println!("{} 🚚 Reconciling changes", style("[4/9]").dim().bold());
        Ok(Reconciler::reconcile(previous, current)?)
    }

    async fn execute_sync_operations(
        &self,
        actions: Vec<Action>,
        final_checksum_tree: ChecksumTree,
    ) -> Result<SyncStats, Box<dyn Error + Send + Sync + 'static>> {
        println!(
            "{} 🚀 Executing {} action(s)",
            style("[5/9]").dim().bold(),
            style(actions.len()).bold()
        );

        let mut stats = SyncStats::default();
        let has_error = Arc::new(AtomicBool::new(false));

        // Phase 4a: Create directories
        stats.directories_created = self
            .execute_directory_operations(&actions, &has_error)
            .await?;

        // Phase 4b: Upload files
        let upload_stats = self
            .execute_file_uploads(&actions, &final_checksum_tree, &has_error)
            .await?;
        stats.files_uploaded = upload_stats.files_uploaded;
        stats.bytes_transferred = upload_stats.bytes_transferred;

        // Phase 4c: Remove files
        if !self.args.skip_removal {
            stats.files_removed = self.execute_file_removals(&actions, &has_error).await?;
        } else {
            println!(
                "{} 🧻 Removing files (skipping)",
                style("[8/9]").dim().bold()
            );
        }

        // Phase 4d: Upload final checksum
        self.upload_final_checksum(&final_checksum_tree).await?;

        stats.had_errors = has_error.load(SeqCst);
        if stats.had_errors {
            return Err("Some operations failed during sync".into());
        }

        Ok(stats)
    }

    async fn execute_directory_operations(
        &self,
        actions: &[Action],
        has_error: &Arc<AtomicBool>,
    ) -> Result<usize, Box<dyn Error + Send + Sync + 'static>> {
        println!("{} 📂 Creating directories", style("[6/9]").dim().bold());

        let create_actions: Vec<_> = actions
            .iter()
            .filter(|action| matches!(action, Action::Mkdir(_)))
            .collect();

        let mut transport = self.create_transport().await?;

        for (i, action) in create_actions.iter().enumerate() {
            if i < self.args.skip {
                continue;
            }

            let start_time = std::time::Instant::now();
            match action {
                Action::Mkdir(path) => match transport.mkdir(path.as_path()).await {
                    Ok(_) => println!(
                        "✅ Creating directory {}/{} {:?} in {:.2?}s",
                        i + 1,
                        create_actions.len(),
                        path,
                        start_time.elapsed().as_secs_f64(),
                    ),
                    Err(error) => {
                        eprintln!(
                            "❌ Error while creating directory {}/{} {:?}: {}",
                            i + 1,
                            create_actions.len(),
                            path,
                            error
                        );
                        has_error.store(true, SeqCst);
                    }
                },
                _ => unreachable!(),
            }
        }

        Ok(create_actions.len())
    }

    async fn execute_file_uploads(
        &self,
        actions: &[Action],
        checksum_tree: &ChecksumTree,
        has_error: &Arc<AtomicBool>,
    ) -> Result<UploadStats, Box<dyn Error + Send + Sync + 'static>> {
        let put_actions: Vec<_> = actions
            .iter()
            .filter(|action| matches!(action, Action::Put(_)))
            .cloned()
            .collect();

        if put_actions.is_empty() {
            return Ok(UploadStats::default());
        }

        // Sort by file size (smallest first for better progress)
        let mut sorted_actions = put_actions.clone();
        sorted_actions.sort_by(|a, b| {
            let Action::Put(a) = a else { unreachable!() };
            let Action::Put(b) = b else { unreachable!() };
            std::fs::metadata(a)
                .unwrap()
                .len()
                .cmp(&std::fs::metadata(b).unwrap().len())
        });

        let total_bytes = Arc::new(AtomicU64::new(
            sorted_actions
                .iter()
                .map(|action| {
                    let Action::Put(path) = action else {
                        unreachable!()
                    };
                    std::fs::metadata(path).unwrap().len()
                })
                .sum::<u64>(),
        ));

        println!(
            "{} 🏂 Uploading {} files ({})",
            style("[7/9]").dim().bold(),
            sorted_actions.len(),
            total_bytes.load(SeqCst).to_human_size()
        );

        let uploaded_bytes = Arc::new(AtomicU64::new(0));
        let progress_bars = Arc::new(MultiProgress::new());
        let checksum_path = Arc::new(PathBuf::from(&self.args.checksum_file));
        let finished_paths = Arc::new(Mutex::new(HashSet::new()));

        // Create transport pool
        let transport_pool = Arc::new(Mutex::new(self.create_transport_pool().await?));

        let sorted_actions_len = sorted_actions.len();
        let upload_tasks = sorted_actions
            .into_iter()
            .enumerate()
            .skip(self.args.skip)
            .map(|(i, action)| {
                let action = action.clone();
                let total_bytes = Arc::clone(&total_bytes);
                let uploaded_bytes = Arc::clone(&uploaded_bytes);
                let progress_bars = Arc::clone(&progress_bars);
                let transport_pool = Arc::clone(&transport_pool);
                let has_error = Arc::clone(has_error);
                let checksum_file = self.args.checksum_file.clone();
                let checksum_path = Arc::clone(&checksum_path);
                let finished_paths = Arc::clone(&finished_paths);
                let intermittent_upload_interval = self.args.intermittent_checksum_upload;
                let checksum_tree = checksum_tree.clone();

                tokio::spawn(async move {
                    Self::upload_single_file(
                        action,
                        i,
                        sorted_actions_len,
                        total_bytes,
                        uploaded_bytes,
                        progress_bars,
                        transport_pool,
                        has_error,
                        checksum_file,
                        checksum_path,
                        finished_paths,
                        intermittent_upload_interval,
                        checksum_tree,
                    )
                    .await
                })
            });

        let results = stream::iter(upload_tasks)
            .buffer_unordered(self.args.concurrency)
            .collect::<Vec<_>>()
            .await;

        let mut upload_stats = UploadStats::default();
        for result in results {
            match result? {
                Ok(bytes) => {
                    upload_stats.files_uploaded += 1;
                    upload_stats.bytes_transferred += bytes;
                }
                Err(_) => {
                    // Error already logged in upload_single_file
                }
            }
        }

        Ok(upload_stats)
    }

    async fn upload_single_file(
        action: Action,
        index: usize,
        total_files: usize,
        total_bytes: Arc<AtomicU64>,
        uploaded_bytes: Arc<AtomicU64>,
        progress_bars: Arc<MultiProgress>,
        transport_pool: Arc<Mutex<Vec<Box<dyn Transport + Send + Sync>>>>,
        has_error: Arc<AtomicBool>,
        _checksum_file: String,
        checksum_path: Arc<PathBuf>,
        finished_paths: Arc<Mutex<HashSet<PathBuf>>>,
        intermittent_upload_interval: usize,
        checksum_tree: ChecksumTree,
    ) -> Result<u64, Box<dyn Error + Send + Sync + 'static>> {
        let Action::Put(path) = action else {
            unreachable!()
        };

        let file = fs::File::open(&path).await?;
        let metadata = file.metadata().await?;
        let file_size = metadata.len();

        // Get transport from pool
        let mut transport = {
            let mut pool = transport_pool.lock().await;
            pool.pop().ok_or("No transport available")?
        };

        // Setup progress bar
        let pb = ProgressBar::new(file_size);
        let pb = Arc::new(progress_bars.add(pb));

        let mut template = format!("[{}/{}] ", index + 1, total_files);
        template.push_str("[{elapsed_precise}] {wide_bar:.cyan/blue} {bytes}/{total_bytes} [{bytes_per_sec}] {msg}");

        pb.set_style(
            ProgressStyle::with_template(&template)
                .unwrap()
                .progress_chars(PROGRESS_BAR_CHARS),
        );

        pb.set_message(path.to_string_lossy().to_string());

        // Create progress-tracking file reader
        let pb_inner = Arc::clone(&pb);
        let progress_file = progress::ProgressStream::new(
            file,
            Box::new(move |uploaded| {
                pb_inner.set_position(uploaded);
            }),
        );

        // Upload the file
        match transport
            .write(path.as_path(), Box::new(progress_file), file_size)
            .await
        {
            Ok(bytes_written) => {
                uploaded_bytes.fetch_add(bytes_written, SeqCst);
                finished_paths.lock().await.insert(path.clone());

                let remaining = total_bytes.load(SeqCst) - uploaded_bytes.load(SeqCst);
                let message = format!(
                    "{} | {} remaining",
                    path.to_string_lossy(),
                    remaining.to_human_size()
                );

                pb.finish_with_message(message.clone());

                // Print success message in CI
                if std::env::var("CI").is_ok() {
                    println!("✅ {}", message);
                }

                // Handle intermittent checksum upload
                let finished_count = finished_paths.lock().await.len();
                if intermittent_upload_interval > 0
                    && finished_count > 0
                    && finished_count % intermittent_upload_interval == 0
                {
                    pb.set_message("📸 Uploading intermittent checksum");
                    if let Err(e) = transport
                        .write_last_checksum(checksum_path.as_path(), &checksum_tree)
                        .await
                    {
                        pb.set_message(format!("❌ Error uploading intermittent checksum: {}", e));
                    } else {
                        pb.set_message(message);
                    }
                }

                // Return transport to pool
                transport_pool.lock().await.push(transport);
                Ok(bytes_written)
            }
            Err(error) => {
                let message = format!("❌ Error while uploading {:?}: {}", path, error);
                pb.abandon_with_message(message.clone());
                has_error.store(true, SeqCst);

                if std::env::var("CI").is_ok() {
                    println!("{}", message);
                }

                // Return transport to pool even on error
                transport_pool.lock().await.push(transport);
                Err(error)
            }
        }
    }

    async fn execute_file_removals(
        &self,
        actions: &[Action],
        has_error: &Arc<AtomicBool>,
    ) -> Result<usize, Box<dyn Error + Send + Sync + 'static>> {
        println!("{} 🧻 Removing files", style("[8/9]").dim().bold());

        let remove_actions: Vec<_> = actions
            .iter()
            .filter(|action| matches!(action, Action::Remove(_)))
            .cloned()
            .collect();

        let transport_pool = Arc::new(Mutex::new(self.create_transport_pool().await?));

        let removal_tasks = remove_actions
            .iter()
            .enumerate()
            .skip(self.args.skip)
            .map(|(i, action)| {
                let action = action.clone();
                let transport_pool = Arc::clone(&transport_pool);
                let has_error = Arc::clone(has_error);
                let total_removals = remove_actions.len();

                tokio::spawn(async move {
                    let mut transport = {
                        let mut pool = transport_pool.lock().await;
                        pool.pop().ok_or("No transport available")?
                    };

                    let start_time = std::time::Instant::now();
                    let result = match action {
                        Action::Remove(path) => match transport.remove(path.as_path()).await {
                            Ok(_) => {
                                println!(
                                    "✅ Removed {}/{} file: {:?} in {:.2?}s",
                                    i + 1,
                                    total_removals,
                                    path,
                                    start_time.elapsed().as_secs_f64(),
                                );
                                Ok(())
                            }
                            Err(error) => {
                                eprintln!("❌ Error while removing {:?}: {}", path, error);
                                has_error.store(true, SeqCst);
                                Err(error)
                            }
                        },
                        _ => unreachable!(),
                    };

                    // Return transport to pool
                    transport_pool.lock().await.push(transport);
                    result
                })
            });

        let results = stream::iter(removal_tasks)
            .buffer_unordered(self.args.concurrency)
            .collect::<Vec<_>>()
            .await;

        let mut removed_count = 0;
        for result in results {
            match result? {
                Ok(_) => removed_count += 1,
                Err(_) => {
                    // Error already logged
                }
            }
        }

        Ok(removed_count)
    }

    async fn upload_final_checksum(
        &self,
        checksum_tree: &ChecksumTree,
    ) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
        println!("{} 🏁 Uploading checksum", style("[9/9]").dim().bold());

        let mut transport = self.create_transport().await?;
        transport
            .write_last_checksum(Path::new(&self.args.checksum_file), checksum_tree)
            .await?;
        transport.close().await?;

        Ok(())
    }

    async fn create_transport(
        &self,
    ) -> Result<Box<dyn Transport + Send + Sync>, Box<dyn Error + Send + Sync + 'static>> {
        if self.args.enable_retry_transport {
            self.create_single_retry_transport().await
        } else {
            self.create_single_base_transport().await
        }
    }

    async fn create_single_retry_transport(
        &self,
    ) -> Result<Box<dyn Transport + Send + Sync>, Box<dyn Error + Send + Sync + 'static>> {
        let factory = Arc::new(ConfigBasedTransportFactory::new(self.args.clone()));
        let retry_config = RetryConfig::from_args(&self.args);
        let retry_transport = RetryTransport::new(factory, retry_config);
        Ok(Box::new(retry_transport) as Box<dyn Transport + Send + Sync>)
    }

    async fn create_single_base_transport(
        &self,
    ) -> Result<Box<dyn Transport + Send + Sync>, Box<dyn Error + Send + Sync + 'static>> {
        let factory = ConfigBasedTransportFactory::new(self.args.clone());
        factory.create().await
    }

    async fn create_transport_pool(
        &self,
    ) -> Result<Vec<Box<dyn Transport + Send + Sync>>, Box<dyn Error + Send + Sync + 'static>> {
        let mut transports = Vec::new();
        for _ in 0..self.args.concurrency {
            transports.push(self.create_transport().await?);
        }
        Ok(transports)
    }
}

#[derive(Debug, Default)]
struct SyncStats {
    files_uploaded: usize,
    bytes_transferred: u64,
    files_removed: usize,
    directories_created: usize,
    had_errors: bool,
}

#[derive(Debug, Default)]
struct UploadStats {
    files_uploaded: usize,
    bytes_transferred: u64,
}
