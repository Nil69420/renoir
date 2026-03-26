use clap::{Arg, Command};
use renoir::{
    allocators::{Allocator, BumpAllocator, PoolAllocator},
    buffers::{BufferPool, BufferPoolConfig},
    large_payloads::{
        BlobDescriptor, BlobManager, ChunkManager, ChunkingStrategy, EpochReclaimer,
        LaserScanHeader, PointCloudHeader, ROS2MessageManager, ReclamationPolicy,
    },
    memory::{BackingType, RegionConfig, SharedMemoryManager},
    ringbuf::RingBuffer,
    shared_pools::SharedBufferPoolManager,
    Result,
};
use std::{path::PathBuf, sync::Arc, time::Duration};

fn main() -> Result<()> {
    env_logger::init();

    let matches = Command::new("renoir-cli")
        .version(env!("CARGO_PKG_VERSION"))
        .about("Renoir Shared Memory Manager CLI Tool")
        .subcommand(
            Command::new("region")
                .about("Manage shared memory regions")
                .subcommand(
                    Command::new("create")
                        .about("Create a new shared memory region")
                        .arg(
                            Arg::new("name")
                                .short('n')
                                .long("name")
                                .value_name("NAME")
                                .help("Name of the region")
                                .required(true),
                        )
                        .arg(
                            Arg::new("size")
                                .short('s')
                                .long("size")
                                .value_name("SIZE")
                                .help("Size in bytes")
                                .required(true),
                        )
                        .arg(
                            Arg::new("file")
                                .short('f')
                                .long("file")
                                .value_name("FILE")
                                .help("File path for file-backed region"),
                        )
                        .arg(
                            Arg::new("memfd")
                                .long("memfd")
                                .num_args(0)
                                .help("Use anonymous memory file descriptor (Linux only)"),
                        ),
                )
                .subcommand(Command::new("list").about("List existing regions")),
        )
        .subcommand(
            Command::new("buffer")
                .about("Buffer pool operations")
                .subcommand(
                    Command::new("test")
                        .about("Test buffer pool performance")
                        .arg(
                            Arg::new("region")
                                .short('r')
                                .long("region")
                                .value_name("REGION")
                                .help("Region name")
                                .required(true),
                        )
                        .arg(
                            Arg::new("buffer_size")
                                .short('b')
                                .long("buffer-size")
                                .value_name("SIZE")
                                .help("Buffer size")
                                .default_value("4096"),
                        )
                        .arg(
                            Arg::new("count")
                                .short('c')
                                .long("count")
                                .value_name("COUNT")
                                .help("Number of operations")
                                .default_value("1000"),
                        ),
                ),
        )
        .subcommand(
            Command::new("allocator")
                .about("Allocator testing")
                .subcommand(
                    Command::new("bump").about("Test bump allocator").arg(
                        Arg::new("size")
                            .short('s')
                            .long("size")
                            .value_name("SIZE")
                            .help("Memory size")
                            .default_value("1048576"),
                    ),
                )
                .subcommand(
                    Command::new("pool")
                        .about("Test pool allocator")
                        .arg(
                            Arg::new("size")
                                .short('s')
                                .long("size")
                                .value_name("SIZE")
                                .help("Memory size")
                                .default_value("1048576"),
                        )
                        .arg(
                            Arg::new("block_size")
                                .short('b')
                                .long("block-size")
                                .value_name("BLOCK_SIZE")
                                .help("Block size")
                                .default_value("256"),
                        ),
                ),
        )
        .subcommand(
            Command::new("ringbuf")
                .about("Ring buffer testing")
                .arg(
                    Arg::new("capacity")
                        .short('c')
                        .long("capacity")
                        .value_name("CAPACITY")
                        .help("Ring buffer capacity (must be power of 2)")
                        .default_value("1024"),
                )
                .arg(
                    Arg::new("operations")
                        .short('o')
                        .long("operations")
                        .value_name("OPS")
                        .help("Number of operations")
                        .default_value("10000"),
                ),
        )
        .subcommand(
            Command::new("large-payloads")
                .about("Test large payloads system for ROS2 messages")
                .subcommand(
                    Command::new("blob")
                        .about("Test blob management")
                        .arg(
                            Arg::new("size")
                                .short('s')
                                .long("size")
                                .value_name("SIZE")
                                .help("Blob size in bytes")
                                .default_value("1048576"),
                        )
                        .arg(
                            Arg::new("count")
                                .short('c')
                                .long("count")
                                .value_name("COUNT")
                                .help("Number of blobs to create")
                                .default_value("10"),
                        ),
                )
                .subcommand(
                    Command::new("ros2")
                        .about("Test ROS2 message types")
                        .arg(
                            Arg::new("type")
                                .short('t')
                                .long("type")
                                .value_name("TYPE")
                                .help("Message type (image, pointcloud, laserscan)")
                                .default_value("image"),
                        )
                        .arg(
                            Arg::new("width")
                                .long("width")
                                .value_name("WIDTH")
                                .help("Image width (for image type)")
                                .default_value("1920"),
                        )
                        .arg(
                            Arg::new("height")
                                .long("height")
                                .value_name("HEIGHT")
                                .help("Image height (for image type)")
                                .default_value("1080"),
                        ),
                )
                .subcommand(
                    Command::new("chunking")
                        .about("Test chunking system for oversized payloads")
                        .arg(
                            Arg::new("payload_size")
                                .short('p')
                                .long("payload-size")
                                .value_name("SIZE")
                                .help("Total payload size")
                                .default_value("16777216"),
                        )
                        .arg(
                            Arg::new("chunk_size")
                                .short('c')
                                .long("chunk-size")
                                .value_name("SIZE")
                                .help("Individual chunk size")
                                .default_value("1048576"),
                        ),
                )
                .subcommand(
                    Command::new("reclamation")
                        .about("Test epoch-based memory reclamation")
                        .arg(
                            Arg::new("objects")
                                .short('o')
                                .long("objects")
                                .value_name("COUNT")
                                .help("Number of objects to allocate and reclaim")
                                .default_value("100"),
                        ),
                ),
        )
        .subcommand(Command::new("info").about("Show version and build information"))
        .get_matches();

    match matches.subcommand() {
        Some(("region", region_matches)) => handle_region_commands(region_matches),
        Some(("buffer", buffer_matches)) => handle_buffer_commands(buffer_matches),
        Some(("allocator", alloc_matches)) => handle_allocator_commands(alloc_matches),
        Some(("ringbuf", ring_matches)) => handle_ringbuf_commands(ring_matches),
        Some(("large-payloads", large_matches)) => handle_large_payloads_commands(large_matches),
        Some(("info", _)) => show_info(),
        _ => {
            println!("Use --help for usage information");
            Ok(())
        }
    }
}

fn handle_region_commands(matches: &clap::ArgMatches) -> Result<()> {
    match matches.subcommand() {
        Some(("create", create_matches)) => {
            let name = create_matches.get_one::<String>("name").unwrap();
            let size: usize = create_matches
                .get_one::<String>("size")
                .unwrap()
                .parse()
                .map_err(|_| {
                    renoir::error::RenoirError::invalid_parameter("size", "Invalid size format")
                })?;

            let backing_type = if create_matches.get_flag("memfd") {
                #[cfg(target_os = "linux")]
                {
                    BackingType::MemFd
                }
                #[cfg(not(target_os = "linux"))]
                {
                    return Err(renoir::error::RenoirError::platform(
                        "memfd not supported on this platform",
                    ));
                }
            } else {
                BackingType::FileBacked
            };

            let file_path = create_matches
                .get_one::<String>("file")
                .map(PathBuf::from)
                .or_else(|| Some(PathBuf::from(format!("/tmp/renoir_{}", name))));

            let manager = SharedMemoryManager::new();
            let config = RegionConfig {
                name: name.to_string(),
                size,
                backing_type,
                file_path,
                create: true,
                permissions: 0o644,
            };

            let region = manager.create_region(config)?;
            println!(
                "Created region '{}' with size {} bytes",
                region.name(),
                region.size()
            );
        }
        Some(("list", _)) => {
            let manager = SharedMemoryManager::new();
            let regions = manager.list_regions();
            if regions.is_empty() {
                println!("No regions found");
            } else {
                println!("Regions:");
                for region_name in regions {
                    println!("  - {}", region_name);
                }
            }
        }
        _ => println!("Use 'region --help' for usage information"),
    }
    Ok(())
}

fn handle_buffer_commands(matches: &clap::ArgMatches) -> Result<()> {
    match matches.subcommand() {
        Some(("test", test_matches)) => {
            let region_name = test_matches.get_one::<String>("region").unwrap();
            let buffer_size: usize = test_matches
                .get_one::<String>("buffer_size")
                .unwrap()
                .parse()
                .map_err(|_| {
                    renoir::error::RenoirError::invalid_parameter("buffer_size", "Invalid size")
                })?;
            let count: usize = test_matches
                .get_one::<String>("count")
                .unwrap()
                .parse()
                .map_err(|_| {
                    renoir::error::RenoirError::invalid_parameter("count", "Invalid count")
                })?;

            println!("Testing buffer pool performance...");
            println!("Region: {}", region_name);
            println!("Buffer size: {} bytes", buffer_size);
            println!("Operations: {}", count);

            let manager = SharedMemoryManager::new();

            let region_config = RegionConfig {
                name: region_name.to_string(),
                size: buffer_size * count * 2,
                backing_type: BackingType::FileBacked,
                file_path: Some(PathBuf::from(format!("/tmp/renoir_{}", region_name))),
                create: true,
                permissions: 0o644,
            };

            let region = manager.create_region(region_config)?;

            let pool_config = BufferPoolConfig {
                name: "test_pool".to_string(),
                buffer_size,
                initial_count: std::cmp::min(count / 4, 100),
                max_count: count,
                alignment: 64,
                pre_allocate: true,
                allocation_timeout: Some(Duration::from_millis(100)),
            };

            let buffer_pool = BufferPool::new(pool_config, region)?;

            let start = std::time::Instant::now();

            for i in 0..count {
                let buffer = buffer_pool.get_buffer()?;

                let test_data = format!("Test data {}", i);
                let bytes = test_data.as_bytes();
                if bytes.len() <= buffer.size() {
                    // Safety: we verified the data fits within the buffer bounds
                    unsafe {
                        std::ptr::copy_nonoverlapping(
                            bytes.as_ptr(),
                            buffer.as_ptr() as *mut u8,
                            bytes.len(),
                        );
                    }
                }

                buffer_pool.return_buffer(buffer)?;
            }

            let elapsed = start.elapsed();
            let ops_per_sec = count as f64 / elapsed.as_secs_f64();

            println!("\nResults:");
            println!("  Total time: {:.2}ms", elapsed.as_millis());
            println!("  Operations/sec: {:.0}", ops_per_sec);
            println!(
                "  Average latency: {:.2}us",
                elapsed.as_micros() as f64 / count as f64
            );

            let stats = buffer_pool.stats();
            println!("  Success rate: {:.2}%", stats.success_rate() * 100.0);
        }
        _ => println!("Use 'buffer --help' for usage information"),
    }
    Ok(())
}

fn handle_allocator_commands(matches: &clap::ArgMatches) -> Result<()> {
    match matches.subcommand() {
        Some(("bump", bump_matches)) => {
            let size: usize = bump_matches
                .get_one::<String>("size")
                .unwrap()
                .parse()
                .map_err(|_| {
                    renoir::error::RenoirError::invalid_parameter("size", "Invalid size")
                })?;

            println!("Testing bump allocator with {} bytes", size);

            let mut memory = vec![0u8; size];
            let allocator = BumpAllocator::new(&mut memory)?;

            println!("Initial state:");
            println!("  Total size: {}", allocator.total_size());
            println!("  Used size: {}", allocator.used_size());

            let start = std::time::Instant::now();
            let mut allocations = Vec::new();

            for i in 1..=100 {
                let alloc_size = 64 * i;
                if let Ok(ptr) = allocator.allocate(alloc_size, 8) {
                    allocations.push((ptr, alloc_size));
                } else {
                    break;
                }
            }

            let elapsed = start.elapsed();

            println!("\nAfter allocations:");
            println!("  Allocations made: {}", allocations.len());
            println!("  Used size: {}", allocator.used_size());
            println!("  Available size: {}", allocator.available_size());
            println!("  Time taken: {:.2}us", elapsed.as_micros());
            println!(
                "  Avg time per allocation: {:.2}us",
                elapsed.as_micros() as f64 / allocations.len() as f64
            );
        }
        Some(("pool", pool_matches)) => {
            let size: usize = pool_matches
                .get_one::<String>("size")
                .unwrap()
                .parse()
                .map_err(|_| {
                    renoir::error::RenoirError::invalid_parameter("size", "Invalid size")
                })?;
            let block_size: usize = pool_matches
                .get_one::<String>("block_size")
                .unwrap()
                .parse()
                .map_err(|_| {
                    renoir::error::RenoirError::invalid_parameter(
                        "block_size",
                        "Invalid block size",
                    )
                })?;

            println!(
                "Testing pool allocator with {} bytes, block size {}",
                size, block_size
            );

            let mut memory = vec![0u8; size];
            let allocator = PoolAllocator::new(&mut memory, block_size)?;

            println!("Initial state:");
            println!("  Total size: {}", allocator.total_size());
            println!("  Used size: {}", allocator.used_size());

            let start = std::time::Instant::now();
            let mut allocations = Vec::new();

            while let Ok(ptr) = allocator.allocate(block_size, 8) {
                allocations.push(ptr);
            }

            let alloc_time = start.elapsed();

            println!("\nAfter allocations:");
            println!("  Blocks allocated: {}", allocations.len());
            println!("  Used size: {}", allocator.used_size());
            println!("  Allocation time: {:.2}us", alloc_time.as_micros());

            let start = std::time::Instant::now();
            for ptr in allocations {
                allocator.deallocate(ptr, block_size)?;
            }
            let dealloc_time = start.elapsed();

            println!("\nAfter deallocations:");
            println!("  Used size: {}", allocator.used_size());
            println!("  Deallocation time: {:.2}us", dealloc_time.as_micros());
        }
        _ => println!("Use 'allocator --help' for usage information"),
    }
    Ok(())
}

fn handle_ringbuf_commands(matches: &clap::ArgMatches) -> Result<()> {
    let capacity: usize = matches
        .get_one::<String>("capacity")
        .unwrap()
        .parse()
        .map_err(|_| {
            renoir::error::RenoirError::invalid_parameter("capacity", "Invalid capacity")
        })?;

    let operations: usize = matches
        .get_one::<String>("operations")
        .unwrap()
        .parse()
        .map_err(|_| {
            renoir::error::RenoirError::invalid_parameter("operations", "Invalid operations count")
        })?;

    if !capacity.is_power_of_two() {
        return Err(renoir::error::RenoirError::invalid_parameter(
            "capacity",
            "Capacity must be a power of 2",
        ));
    }

    println!(
        "Testing ring buffer with capacity {} and {} operations",
        capacity, operations
    );

    let buffer: RingBuffer<u64> = RingBuffer::new(capacity)?;
    let producer = buffer.producer();
    let consumer = buffer.consumer();

    println!("Initial state:");
    println!("  Capacity: {}", buffer.capacity());
    println!("  Length: {}", buffer.len());
    println!("  Is empty: {}", buffer.is_empty());

    let start = std::time::Instant::now();
    let mut pushed = 0;

    for i in 0..operations {
        match producer.try_push(i as u64) {
            Ok(()) => pushed += 1,
            Err(_) => {
                for _ in 0..capacity / 2 {
                    if consumer.try_pop().is_err() {
                        break;
                    }
                }
            }
        }
    }

    let push_time = start.elapsed();

    let start = std::time::Instant::now();
    let mut consumed = 0;

    while consumer.try_pop().is_ok() {
        consumed += 1;
    }

    let pop_time = start.elapsed();

    println!("\nResults:");
    println!("  Items pushed: {}", pushed);
    println!("  Items consumed: {}", consumed);
    println!("  Push time: {:.2}us", push_time.as_micros());
    println!("  Pop time: {:.2}us", pop_time.as_micros());
    println!(
        "  Push rate: {:.0} ops/sec",
        pushed as f64 / push_time.as_secs_f64()
    );
    println!(
        "  Pop rate: {:.0} ops/sec",
        consumed as f64 / pop_time.as_secs_f64()
    );

    Ok(())
}

fn handle_large_payloads_commands(matches: &clap::ArgMatches) -> Result<()> {
    match matches.subcommand() {
        Some(("blob", blob_matches)) => {
            let size: usize = blob_matches
                .get_one::<String>("size")
                .unwrap()
                .parse()
                .map_err(|_| {
                    renoir::error::RenoirError::invalid_parameter("size", "Invalid size format")
                })?;
            let count: usize = blob_matches
                .get_one::<String>("count")
                .unwrap()
                .parse()
                .map_err(|_| {
                    renoir::error::RenoirError::invalid_parameter("count", "Invalid count format")
                })?;

            println!("Testing blob management...");
            println!("Blob size: {} bytes", size);
            println!("Blob count: {}", count);

            let blob_manager = BlobManager::new();

            let start = std::time::Instant::now();
            let mut blob_handles = Vec::new();

            for i in 0..count {
                let test_data = vec![i as u8; size];
                let header = blob_manager.create_header(
                    renoir::large_payloads::content_types::CUSTOM_BINARY,
                    &test_data,
                );
                let descriptor = BlobDescriptor::new(i as u32, i as u32, header);
                blob_handles.push(descriptor);
            }

            let creation_time = start.elapsed();

            let start = std::time::Instant::now();
            for blob_desc in &blob_handles {
                if blob_desc.payload_size() as usize != size {
                    return Err(renoir::error::RenoirError::invalid_parameter(
                        "blob",
                        "Blob size mismatch",
                    ));
                }
            }

            let validation_time = start.elapsed();

            println!("\nResults:");
            println!("  Blob creation: {:.2}ms", creation_time.as_millis());
            println!("  Blob validation: {:.2}ms", validation_time.as_millis());
            println!(
                "  Creation rate: {:.0} blobs/sec",
                count as f64 / creation_time.as_secs_f64()
            );
            println!(
                "  Total memory used: {:.2}MB",
                (size * count) as f64 / 1024.0 / 1024.0
            );
        }
        Some(("ros2", ros2_matches)) => {
            let msg_type = ros2_matches.get_one::<String>("type").unwrap();

            println!("Testing ROS2 message type: {}", msg_type);

            let pool_manager = Arc::new(SharedBufferPoolManager::new());
            let chunking_strategy = ChunkingStrategy::FixedSize(1024 * 1024);
            let ros2_manager = ROS2MessageManager::new(pool_manager, chunking_strategy);

            match msg_type.as_str() {
                "image" => {
                    let width: u32 = ros2_matches
                        .get_one::<String>("width")
                        .unwrap()
                        .parse()
                        .map_err(|_| {
                            renoir::error::RenoirError::invalid_parameter("width", "Invalid width")
                        })?;
                    let height: u32 = ros2_matches
                        .get_one::<String>("height")
                        .unwrap()
                        .parse()
                        .map_err(|_| {
                            renoir::error::RenoirError::invalid_parameter(
                                "height",
                                "Invalid height",
                            )
                        })?;

                    let data_size = (width * height * 3) as usize;
                    let image_data = vec![128u8; data_size];

                    println!(
                        "Creating {}x{} RGB image ({:.2}MB)",
                        width,
                        height,
                        data_size as f64 / 1024.0 / 1024.0
                    );

                    let start = std::time::Instant::now();

                    let pool_id: u32 = 1;

                    let msg = ros2_manager.create_image_message(
                        width,
                        height,
                        "rgb8",
                        &image_data,
                        pool_id,
                    )?;
                    let creation_time = start.elapsed();

                    println!("  Message created in {:.2}ms", creation_time.as_millis());
                    let w = msg.header.width;
                    let h = msg.header.height;
                    println!("  Image dimensions: {}x{}", w, h);
                }
                "pointcloud" => {
                    let num_points: usize = 100000;
                    let header =
                        PointCloudHeader::new(num_points as u32, 16, num_points as u32, 1, true);

                    let data_size = num_points * 16;

                    println!(
                        "Creating point cloud with {} points ({:.2}MB)",
                        num_points,
                        data_size as f64 / 1024.0 / 1024.0
                    );

                    let start = std::time::Instant::now();
                    println!(
                        "  Point cloud header created in {:.2}us",
                        start.elapsed().as_micros()
                    );
                    let pc = header.point_count;
                    let ps = header.point_step;
                    println!("  Point count: {}", pc);
                    println!("  Point step: {} bytes", ps);
                    println!("  Is organized: {}", header.is_organized());
                }
                "laserscan" => {
                    let num_ranges: usize = 360;
                    let header = LaserScanHeader::new(
                        -std::f32::consts::PI,
                        std::f32::consts::PI,
                        2.0 * std::f32::consts::PI / num_ranges as f32,
                        0.1,
                        10.0,
                        num_ranges as u32,
                        num_ranges as u32,
                    );

                    println!("Creating laser scan with {} ranges", num_ranges);

                    let start = std::time::Instant::now();
                    println!(
                        "  Laser scan header created in {:.2}us",
                        start.elapsed().as_micros()
                    );
                    let rc = header.range_count;
                    let ic = header.intensity_count;
                    println!("  Range count: {}", rc);
                    println!("  Intensity count: {}", ic);
                    println!(
                        "  Angular resolution: {:.4} degrees",
                        header.angle_increment.to_degrees()
                    );
                }
                _ => {
                    return Err(renoir::error::RenoirError::invalid_parameter(
                        "type",
                        "Unknown message type",
                    ));
                }
            }
        }
        Some(("chunking", chunk_matches)) => {
            let payload_size: usize = chunk_matches
                .get_one::<String>("payload_size")
                .unwrap()
                .parse()
                .map_err(|_| {
                    renoir::error::RenoirError::invalid_parameter("payload_size", "Invalid size")
                })?;
            let chunk_size: usize = chunk_matches
                .get_one::<String>("chunk_size")
                .unwrap()
                .parse()
                .map_err(|_| {
                    renoir::error::RenoirError::invalid_parameter("chunk_size", "Invalid size")
                })?;

            println!("Testing chunking system...");
            println!(
                "Payload size: {:.2}MB",
                payload_size as f64 / 1024.0 / 1024.0
            );
            println!("Chunk size: {:.2}KB", chunk_size as f64 / 1024.0);

            let num_chunks = payload_size.div_ceil(chunk_size);
            println!("Expected chunks: {}", num_chunks);

            let pool_manager = Arc::new(SharedBufferPoolManager::new());
            let strategy = ChunkingStrategy::FixedSize(chunk_size);
            let chunk_manager = ChunkManager::new(pool_manager, strategy);

            let blob_manager = BlobManager::new();
            let test_payload: Vec<u8> = (0..payload_size).map(|i| (i % 256) as u8).collect();
            let blob_header = blob_manager.create_header(
                renoir::large_payloads::content_types::CUSTOM_BINARY,
                &test_payload,
            );
            let pool_id: u32 = 0;

            let start = std::time::Instant::now();
            let chunks = chunk_manager.chunk_payload(&test_payload, &blob_header, pool_id)?;
            let chunking_time = start.elapsed();

            println!("\nResults:");
            println!("  Actual chunks created: {}", chunks.len());
            println!("  Chunking time: {:.2}ms", chunking_time.as_millis());
            println!(
                "  Chunking rate: {:.2}MB/s",
                payload_size as f64 / 1024.0 / 1024.0 / chunking_time.as_secs_f64()
            );

            let mut total_size: u32 = 0;
            for (i, chunk) in chunks.iter().enumerate() {
                total_size += chunk.payload_size;
                if i == 0 {
                    println!("  First chunk size: {} bytes", chunk.payload_size);
                }
                if i == chunks.len() - 1 {
                    println!("  Last chunk size: {} bytes", chunk.payload_size);
                }
            }
            println!("  Total reassembled size: {} bytes", total_size);
            println!("  Size matches: {}", total_size as usize == payload_size);
        }
        Some(("reclamation", reclaim_matches)) => {
            let object_count: usize = reclaim_matches
                .get_one::<String>("objects")
                .unwrap()
                .parse()
                .map_err(|_| {
                    renoir::error::RenoirError::invalid_parameter("objects", "Invalid count")
                })?;

            println!("Testing epoch-based reclamation...");
            println!("Objects to create: {}", object_count);

            let policy = ReclamationPolicy {
                max_age: Duration::from_secs(1),
                epoch_advance_interval: Duration::from_millis(100),
                ..Default::default()
            };

            let pool_manager = Arc::new(SharedBufferPoolManager::new());
            let reclaimer = EpochReclaimer::new(pool_manager, policy);

            let start = std::time::Instant::now();

            let blob_manager = BlobManager::new();
            for i in 0..object_count {
                let test_data = vec![0u8; 1024];
                let header = blob_manager.create_header(
                    renoir::large_payloads::content_types::CUSTOM_BINARY,
                    &test_data,
                );
                let descriptor = BlobDescriptor::new(i as u32, i as u32, header);
                reclaimer.mark_for_reclamation(descriptor)?;
            }

            let marking_time = start.elapsed();

            let start = std::time::Instant::now();
            let stats = reclaimer.get_statistics();
            let reclamation_time = start.elapsed();

            println!("\nResults:");
            println!("  Marking time: {:.2}ms", marking_time.as_millis());
            println!("  Reclamation time: {:.2}us", reclamation_time.as_micros());
            println!("  Objects marked: {}", object_count);
            println!(
                "  Items reclaimed: {}",
                stats
                    .items_reclaimed
                    .load(std::sync::atomic::Ordering::Relaxed)
            );
            println!(
                "  Items force-reclaimed: {}",
                stats
                    .items_force_reclaimed
                    .load(std::sync::atomic::Ordering::Relaxed)
            );
        }
        _ => println!("Use 'large-payloads --help' for usage information"),
    }
    Ok(())
}

fn show_info() -> Result<()> {
    println!("Renoir Shared Memory Manager");
    println!("Version: {}", env!("CARGO_PKG_VERSION"));
    println!(
        "Build time: {}",
        option_env!("VERGEN_BUILD_TIMESTAMP").unwrap_or("unknown")
    );
    println!(
        "Git commit: {}",
        option_env!("VERGEN_GIT_SHA").unwrap_or("unknown")
    );
    println!(
        "Rust version: {}",
        option_env!("VERGEN_RUSTC_SEMVER").unwrap_or("unknown")
    );

    println!("\nFeatures:");
    #[cfg(feature = "file-backed")]
    println!("  + File-backed shared memory");

    #[cfg(all(feature = "memfd", target_os = "linux"))]
    println!("  + Anonymous memory file descriptors (memfd)");

    #[cfg(feature = "c-api")]
    println!("  + C API for foreign function interface");

    println!("\nCapabilities:");
    println!("  - Named shared memory regions");
    println!("  - Buffer pools with multiple allocation strategies");
    println!("  - Lock-free ring buffers");
    println!("  - Sequence/version tracking");
    println!("  - Zero-copy data sharing");

    Ok(())
}
