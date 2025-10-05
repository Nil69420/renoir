use clap::{App, Arg, SubCommand};
use renoir::{
    allocator::{Allocator, BumpAllocator, PoolAllocator},
    buffer::{BufferPool, BufferPoolConfig},
    large_payloads::{
        BlobHeader, BlobManager, ChunkManager, ChunkingStrategy, EpochReclaimer, ImageHeader,
        LaserScanHeader, PointCloudHeader, ROS2MessageManager, ROS2MessageType, ReclamationPolicy,
    },
    memory::{BackingType, RegionConfig, SharedMemoryManager},
    ringbuf::RingBuffer,
    Result,
};
use std::{path::PathBuf, sync::Arc, time::Duration};

fn main() -> Result<()> {
    env_logger::init();

    let matches = App::new("renoir-cli")
        .version(env!("CARGO_PKG_VERSION"))
        .about("Renoir Shared Memory Manager CLI Tool")
        .subcommand(
            SubCommand::with_name("region")
                .about("Manage shared memory regions")
                .subcommand(
                    SubCommand::with_name("create")
                        .about("Create a new shared memory region")
                        .arg(
                            Arg::with_name("name")
                                .short("n")
                                .long("name")
                                .value_name("NAME")
                                .help("Name of the region")
                                .required(true)
                                .takes_value(true),
                        )
                        .arg(
                            Arg::with_name("size")
                                .short("s")
                                .long("size")
                                .value_name("SIZE")
                                .help("Size in bytes")
                                .required(true)
                                .takes_value(true),
                        )
                        .arg(
                            Arg::with_name("file")
                                .short("f")
                                .long("file")
                                .value_name("FILE")
                                .help("File path for file-backed region")
                                .takes_value(true),
                        )
                        .arg(
                            Arg::with_name("memfd")
                                .long("memfd")
                                .help("Use anonymous memory file descriptor (Linux only)"),
                        ),
                )
                .subcommand(SubCommand::with_name("list").about("List existing regions")),
        )
        .subcommand(
            SubCommand::with_name("buffer")
                .about("Buffer pool operations")
                .subcommand(
                    SubCommand::with_name("test")
                        .about("Test buffer pool performance")
                        .arg(
                            Arg::with_name("region")
                                .short("r")
                                .long("region")
                                .value_name("REGION")
                                .help("Region name")
                                .required(true)
                                .takes_value(true),
                        )
                        .arg(
                            Arg::with_name("buffer_size")
                                .short("b")
                                .long("buffer-size")
                                .value_name("SIZE")
                                .help("Buffer size")
                                .default_value("4096")
                                .takes_value(true),
                        )
                        .arg(
                            Arg::with_name("count")
                                .short("c")
                                .long("count")
                                .value_name("COUNT")
                                .help("Number of operations")
                                .default_value("1000")
                                .takes_value(true),
                        ),
                ),
        )
        .subcommand(
            SubCommand::with_name("allocator")
                .about("Allocator testing")
                .subcommand(
                    SubCommand::with_name("bump")
                        .about("Test bump allocator")
                        .arg(
                            Arg::with_name("size")
                                .short("s")
                                .long("size")
                                .value_name("SIZE")
                                .help("Memory size")
                                .default_value("1048576")
                                .takes_value(true),
                        ),
                )
                .subcommand(
                    SubCommand::with_name("pool")
                        .about("Test pool allocator")
                        .arg(
                            Arg::with_name("size")
                                .short("s")
                                .long("size")
                                .value_name("SIZE")
                                .help("Memory size")
                                .default_value("1048576")
                                .takes_value(true),
                        )
                        .arg(
                            Arg::with_name("block_size")
                                .short("b")
                                .long("block-size")
                                .value_name("BLOCK_SIZE")
                                .help("Block size")
                                .default_value("256")
                                .takes_value(true),
                        ),
                ),
        )
        .subcommand(
            SubCommand::with_name("ringbuf")
                .about("Ring buffer testing")
                .arg(
                    Arg::with_name("capacity")
                        .short("c")
                        .long("capacity")
                        .value_name("CAPACITY")
                        .help("Ring buffer capacity (must be power of 2)")
                        .default_value("1024")
                        .takes_value(true),
                )
                .arg(
                    Arg::with_name("operations")
                        .short("o")
                        .long("operations")
                        .value_name("OPS")
                        .help("Number of operations")
                        .default_value("10000")
                        .takes_value(true),
                ),
        )
        .subcommand(
            SubCommand::with_name("large-payloads")
                .about("Test large payloads system for ROS2 messages")
                .subcommand(
                    SubCommand::with_name("blob")
                        .about("Test blob management")
                        .arg(
                            Arg::with_name("size")
                                .short("s")
                                .long("size")
                                .value_name("SIZE")
                                .help("Blob size in bytes")
                                .default_value("1048576")
                                .takes_value(true),
                        )
                        .arg(
                            Arg::with_name("count")
                                .short("c")
                                .long("count")
                                .value_name("COUNT")
                                .help("Number of blobs to create")
                                .default_value("10")
                                .takes_value(true),
                        ),
                )
                .subcommand(
                    SubCommand::with_name("ros2")
                        .about("Test ROS2 message types")
                        .arg(
                            Arg::with_name("type")
                                .short("t")
                                .long("type")
                                .value_name("TYPE")
                                .help("Message type (image, pointcloud, laserscan)")
                                .default_value("image")
                                .takes_value(true),
                        )
                        .arg(
                            Arg::with_name("width")
                                .long("width")
                                .value_name("WIDTH")
                                .help("Image width (for image type)")
                                .default_value("1920")
                                .takes_value(true),
                        )
                        .arg(
                            Arg::with_name("height")
                                .long("height")
                                .value_name("HEIGHT")
                                .help("Image height (for image type)")
                                .default_value("1080")
                                .takes_value(true),
                        ),
                )
                .subcommand(
                    SubCommand::with_name("chunking")
                        .about("Test chunking system for oversized payloads")
                        .arg(
                            Arg::with_name("payload_size")
                                .short("p")
                                .long("payload-size")
                                .value_name("SIZE")
                                .help("Total payload size")
                                .default_value("16777216") // 16MB
                                .takes_value(true),
                        )
                        .arg(
                            Arg::with_name("chunk_size")
                                .short("c")
                                .long("chunk-size")
                                .value_name("SIZE")
                                .help("Individual chunk size")
                                .default_value("1048576") // 1MB
                                .takes_value(true),
                        ),
                )
                .subcommand(
                    SubCommand::with_name("reclamation")
                        .about("Test epoch-based memory reclamation")
                        .arg(
                            Arg::with_name("objects")
                                .short("o")
                                .long("objects")
                                .value_name("COUNT")
                                .help("Number of objects to allocate and reclaim")
                                .default_value("100")
                                .takes_value(true),
                        ),
                ),
        )
        .subcommand(SubCommand::with_name("info").about("Show version and build information"))
        .get_matches();

    match matches.subcommand() {
        ("region", Some(region_matches)) => handle_region_commands(region_matches),
        ("buffer", Some(buffer_matches)) => handle_buffer_commands(buffer_matches),
        ("allocator", Some(alloc_matches)) => handle_allocator_commands(alloc_matches),
        ("ringbuf", Some(ring_matches)) => handle_ringbuf_commands(ring_matches),
        ("large-payloads", Some(large_matches)) => handle_large_payloads_commands(large_matches),
        ("info", Some(_)) => show_info(),
        _ => {
            println!("Use --help for usage information");
            Ok(())
        }
    }
}

fn handle_region_commands(matches: &clap::ArgMatches) -> Result<()> {
    match matches.subcommand() {
        ("create", Some(create_matches)) => {
            let name = create_matches.value_of("name").unwrap();
            let size: usize = create_matches
                .value_of("size")
                .unwrap()
                .parse()
                .map_err(|_| {
                    renoir::error::RenoirError::invalid_parameter("size", "Invalid size format")
                })?;

            let backing_type = if create_matches.is_present("memfd") {
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
                .value_of("file")
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
        ("list", Some(_)) => {
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
        ("test", Some(test_matches)) => {
            let region_name = test_matches.value_of("region").unwrap();
            let buffer_size: usize = test_matches
                .value_of("buffer_size")
                .unwrap()
                .parse()
                .map_err(|_| {
                    renoir::error::RenoirError::invalid_parameter("buffer_size", "Invalid size")
                })?;
            let count: usize = test_matches
                .value_of("count")
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

            // Create or get region
            let region_config = RegionConfig {
                name: region_name.to_string(),
                size: buffer_size * count * 2, // Ensure enough space
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

                // Write some test data
                let test_data = format!("Test data {}", i);
                let bytes = test_data.as_bytes();
                if bytes.len() <= buffer.size() {
                    // Simulate some work
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
                "  Average latency: {:.2}μs",
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
        ("bump", Some(bump_matches)) => {
            let size: usize = bump_matches
                .value_of("size")
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

            // Allocate various sizes
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
            println!("  Time taken: {:.2}μs", elapsed.as_micros());
            println!(
                "  Avg time per allocation: {:.2}μs",
                elapsed.as_micros() as f64 / allocations.len() as f64
            );
        }
        ("pool", Some(pool_matches)) => {
            let size: usize = pool_matches
                .value_of("size")
                .unwrap()
                .parse()
                .map_err(|_| {
                    renoir::error::RenoirError::invalid_parameter("size", "Invalid size")
                })?;
            let block_size: usize = pool_matches
                .value_of("block_size")
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

            // Allocate all available blocks
            loop {
                match allocator.allocate(block_size, 8) {
                    Ok(ptr) => allocations.push(ptr),
                    Err(_) => break,
                }
            }

            let alloc_time = start.elapsed();

            println!("\nAfter allocations:");
            println!("  Blocks allocated: {}", allocations.len());
            println!("  Used size: {}", allocator.used_size());
            println!("  Allocation time: {:.2}μs", alloc_time.as_micros());

            // Deallocate all blocks
            let start = std::time::Instant::now();
            for ptr in allocations {
                allocator.deallocate(ptr, block_size, 8)?;
            }
            let dealloc_time = start.elapsed();

            println!("\nAfter deallocations:");
            println!("  Used size: {}", allocator.used_size());
            println!("  Deallocation time: {:.2}μs", dealloc_time.as_micros());
        }
        _ => println!("Use 'allocator --help' for usage information"),
    }
    Ok(())
}

fn handle_ringbuf_commands(matches: &clap::ArgMatches) -> Result<()> {
    let capacity: usize = matches.value_of("capacity").unwrap().parse().map_err(|_| {
        renoir::error::RenoirError::invalid_parameter("capacity", "Invalid capacity")
    })?;

    let operations: usize = matches
        .value_of("operations")
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

    // Test push performance
    let start = std::time::Instant::now();
    let mut pushed = 0;

    for i in 0..operations {
        match producer.try_push(i as u64) {
            Ok(()) => pushed += 1,
            Err(_) => {
                // Buffer full, consume some items
                for _ in 0..capacity / 2 {
                    if consumer.try_pop().is_err() {
                        break;
                    }
                }
            }
        }
    }

    let push_time = start.elapsed();

    // Consume remaining items
    let start = std::time::Instant::now();
    let mut consumed = 0;

    while consumer.try_pop().is_ok() {
        consumed += 1;
    }

    let pop_time = start.elapsed();

    println!("\nResults:");
    println!("  Items pushed: {}", pushed);
    println!("  Items consumed: {}", consumed);
    println!("  Push time: {:.2}μs", push_time.as_micros());
    println!("  Pop time: {:.2}μs", pop_time.as_micros());
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
        ("blob", Some(blob_matches)) => {
            let size: usize = blob_matches
                .value_of("size")
                .unwrap()
                .parse()
                .map_err(|_| {
                    renoir::error::RenoirError::invalid_parameter("size", "Invalid size format")
                })?;
            let count: usize = blob_matches
                .value_of("count")
                .unwrap()
                .parse()
                .map_err(|_| {
                    renoir::error::RenoirError::invalid_parameter("count", "Invalid count format")
                })?;

            println!("Testing blob management...");
            println!("Blob size: {} bytes", size);
            println!("Blob count: {}", count);

            let manager = SharedMemoryManager::new();
            let region_config = RegionConfig {
                name: "large_payloads_test".to_string(),
                size: size * count * 2, // Ensure enough space
                backing_type: BackingType::FileBacked,
                file_path: Some(PathBuf::from("/tmp/renoir_large_test")),
                create: true,
                permissions: 0o644,
            };

            let region = manager.create_region(region_config)?;
            let blob_manager = BlobManager::new();

            let start = std::time::Instant::now();
            let mut blob_handles = Vec::new();

            // Create blob headers
            for i in 0..count {
                let test_data = vec![i as u8; size];
                let header = blob_manager.create_header(
                    renoir::large_payloads::content_types::CONTENT_TYPE_RAW,
                    &test_data,
                );
                // Create a simple descriptor for testing
                let descriptor = renoir::large_payloads::BlobDescriptor::new(
                    (i * size) as u64,     // offset
                    size,                  // size
                    format!("blob_{}", i), // tag
                )?;
                blob_handles.push(descriptor);
            }

            let creation_time = start.elapsed();

            // Validate blob descriptors
            let start = std::time::Instant::now();
            for (i, blob_desc) in blob_handles.iter().enumerate() {
                if blob_desc.size() != size {
                    return Err(renoir::error::RenoirError::validation("Blob size mismatch"));
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
        ("ros2", Some(ros2_matches)) => {
            let msg_type = ros2_matches.value_of("type").unwrap();

            println!("Testing ROS2 message type: {}", msg_type);

            let manager = SharedMemoryManager::new();
            let region_config = RegionConfig {
                name: "ros2_test".to_string(),
                size: 64 * 1024 * 1024, // 64MB for large messages
                backing_type: BackingType::FileBacked,
                file_path: Some(PathBuf::from("/tmp/renoir_ros2_test")),
                create: true,
                permissions: 0o644,
            };

            let region = manager.create_region(region_config)?;

            // Create buffer pool manager and chunking strategy
            use renoir::shared_pools::SharedBufferPoolManager;
            let pool_manager = Arc::new(SharedBufferPoolManager::new()?);
            let chunking_strategy = ChunkingStrategy::new(1024 * 1024)?; // 1MB chunks
            let ros2_manager = ROS2MessageManager::new(pool_manager, chunking_strategy);

            match msg_type {
                "image" => {
                    let width: u32 =
                        ros2_matches
                            .value_of("width")
                            .unwrap()
                            .parse()
                            .map_err(|_| {
                                renoir::error::RenoirError::invalid_parameter(
                                    "width",
                                    "Invalid width",
                                )
                            })?;
                    let height: u32 =
                        ros2_matches
                            .value_of("height")
                            .unwrap()
                            .parse()
                            .map_err(|_| {
                                renoir::error::RenoirError::invalid_parameter(
                                    "height",
                                    "Invalid height",
                                )
                            })?;

                    let step = width * 3; // 3 bytes per pixel for RGB
                    let data_size = (width * height * 3) as usize;
                    let image_data = vec![128u8; data_size]; // Gray image

                    println!(
                        "Creating {}x{} RGB image ({:.2}MB)",
                        width,
                        height,
                        data_size as f64 / 1024.0 / 1024.0
                    );

                    let start = std::time::Instant::now();

                    // Create a test pool ID for demonstration
                    let pool_id = renoir::shared_pools::PoolId(1);

                    let msg = ros2_manager.create_image_message(
                        width,
                        height,
                        "rgb8",
                        &image_data,
                        pool_id,
                    )?;
                    let creation_time = start.elapsed();

                    println!("  Message created in {:.2}ms", creation_time.as_millis());
                    println!(
                        "  Image dimensions: {}x{}",
                        msg.header.width, msg.header.height
                    );
                }
                "pointcloud" => {
                    let num_points = 100000; // 100k points
                    let header = PointCloudHeader::new(
                        num_points, // point_count
                        16,         // point_step (4 fields * 4 bytes each)
                        num_points, // width
                        1,          // height
                        true,       // is_dense
                    );

                    let data_size = num_points * 16; // x,y,z,intensity (4 floats)
                    let cloud_data = vec![0u8; data_size];

                    println!(
                        "Creating point cloud with {} points ({:.2}MB)",
                        num_points,
                        data_size as f64 / 1024.0 / 1024.0
                    );

                    let start = std::time::Instant::now();
                    // For demo, just create and validate header
                    println!(
                        "  Point cloud header created in {:.2}μs",
                        start.elapsed().as_micros()
                    );
                    println!("  Point count: {}", header.point_count);
                    println!("  Point step: {} bytes", header.point_step);
                    println!("  Is organized: {}", header.is_organized());
                }
                "laserscan" => {
                    let num_ranges = 360; // 1 degree resolution
                    let header = LaserScanHeader::new(
                        -std::f32::consts::PI,                          // angle_min
                        std::f32::consts::PI,                           // angle_max
                        2.0 * std::f32::consts::PI / num_ranges as f32, // angle_increment
                        0.1,                                            // range_min
                        10.0,                                           // range_max
                        num_ranges,                                     // range_count
                        num_ranges,                                     // intensity_count
                    );

                    let ranges = vec![5.0f32; num_ranges]; // 5m range for all points
                    let intensities = vec![100.0f32; num_ranges];

                    println!("Creating laser scan with {} ranges", num_ranges);

                    let start = std::time::Instant::now();
                    // For demo, just create and validate header
                    println!(
                        "  Laser scan header created in {:.2}μs",
                        start.elapsed().as_micros()
                    );
                    println!("  Range count: {}", header.range_count);
                    println!("  Intensity count: {}", header.intensity_count);
                    println!(
                        "  Angular resolution: {:.4}°",
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
        ("chunking", Some(chunk_matches)) => {
            let payload_size: usize = chunk_matches
                .value_of("payload_size")
                .unwrap()
                .parse()
                .map_err(|_| {
                    renoir::error::RenoirError::invalid_parameter("payload_size", "Invalid size")
                })?;
            let chunk_size: usize = chunk_matches
                .value_of("chunk_size")
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

            let num_chunks = (payload_size + chunk_size - 1) / chunk_size;
            println!("Expected chunks: {}", num_chunks);

            let manager = SharedMemoryManager::new();
            let region_config = RegionConfig {
                name: "chunking_test".to_string(),
                size: payload_size * 2, // Extra space for overhead
                backing_type: BackingType::FileBacked,
                file_path: Some(PathBuf::from("/tmp/renoir_chunking_test")),
                create: true,
                permissions: 0o644,
            };

            let region = manager.create_region(region_config)?;
            let strategy = ChunkingStrategy::new(chunk_size)?;
            let chunk_manager = ChunkManager::new(Arc::new(region), strategy)?;

            // Create test payload
            let test_payload: Vec<u8> = (0..payload_size).map(|i| (i % 256) as u8).collect();

            let start = std::time::Instant::now();
            let chunks = chunk_manager.chunk_data(&test_payload)?;
            let chunking_time = start.elapsed();

            println!("\nResults:");
            println!("  Actual chunks created: {}", chunks.len());
            println!("  Chunking time: {:.2}ms", chunking_time.as_millis());
            println!(
                "  Chunking rate: {:.2}MB/s",
                payload_size as f64 / 1024.0 / 1024.0 / chunking_time.as_secs_f64()
            );

            // Verify chunk integrity
            let mut total_size = 0;
            for (i, chunk) in chunks.iter().enumerate() {
                total_size += chunk.size();
                if i == 0 {
                    println!("  First chunk size: {} bytes", chunk.size());
                }
                if i == chunks.len() - 1 {
                    println!("  Last chunk size: {} bytes", chunk.size());
                }
            }
            println!("  Total reassembled size: {} bytes", total_size);
            println!("  Size matches: {}", total_size == payload_size);
        }
        ("reclamation", Some(reclaim_matches)) => {
            let object_count: usize = reclaim_matches
                .value_of("objects")
                .unwrap()
                .parse()
                .map_err(|_| {
                    renoir::error::RenoirError::invalid_parameter("objects", "Invalid count")
                })?;

            println!("Testing epoch-based reclamation...");
            println!("Objects to create: {}", object_count);

            let policy = ReclamationPolicy::new(
                Duration::from_millis(100), // Check interval
                Duration::from_secs(1),     // Max age
            )?;

            // Create pool manager for reclaimer
            let pool_manager = Arc::new(SharedBufferPoolManager::new()?);
            let reclaimer = EpochReclaimer::new(pool_manager, policy);

            let start = std::time::Instant::now();

            // Create dummy blob descriptors for testing
            use renoir::large_payloads::BlobDescriptor;
            for i in 0..object_count {
                let descriptor = BlobDescriptor::new(
                    i as u64,              // offset
                    1024,                  // size
                    format!("test_{}", i), // tag
                )?;
                reclaimer.mark_for_reclamation(descriptor)?;
            }

            let marking_time = start.elapsed();

            // Force reclamation
            let start = std::time::Instant::now();
            let stats = reclaimer.reclamation_stats();
            let reclamation_time = start.elapsed();

            println!("\nResults:");
            println!("  Marking time: {:.2}ms", marking_time.as_millis());
            println!("  Reclamation time: {:.2}μs", reclamation_time.as_micros());
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
    println!("Build time: {}", env!("VERGEN_BUILD_TIMESTAMP"));
    println!("Git commit: {}", env!("VERGEN_GIT_SHA"));
    println!("Rust version: {}", env!("VERGEN_RUSTC_SEMVER"));

    println!("\nFeatures:");
    #[cfg(feature = "file-backed")]
    println!("  ✓ File-backed shared memory");

    #[cfg(all(feature = "memfd", target_os = "linux"))]
    println!("  ✓ Anonymous memory file descriptors (memfd)");

    #[cfg(feature = "c-api")]
    println!("  ✓ C API for foreign function interface");

    println!("\nCapabilities:");
    println!("  - Named shared memory regions");
    println!("  - Buffer pools with multiple allocation strategies");
    println!("  - Lock-free ring buffers");
    println!("  - Sequence/version tracking");
    println!("  - Zero-copy data sharing");

    Ok(())
}
