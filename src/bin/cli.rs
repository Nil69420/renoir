use clap::{App, Arg, SubCommand};
use renoir::{
    memory::{SharedMemoryManager, RegionConfig, BackingType},
    buffer::{BufferPool, BufferPoolConfig},
    allocator::{BumpAllocator, PoolAllocator, Allocator},
    ringbuf::RingBuffer,
    Result,
};
use std::{path::PathBuf, time::Duration, sync::Arc};

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
                .subcommand(
                    SubCommand::with_name("list")
                        .about("List existing regions"),
                ),
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
            SubCommand::with_name("info")
                .about("Show version and build information"),
        )
        .get_matches();

    match matches.subcommand() {
        ("region", Some(region_matches)) => handle_region_commands(region_matches),
        ("buffer", Some(buffer_matches)) => handle_buffer_commands(buffer_matches),
        ("allocator", Some(alloc_matches)) => handle_allocator_commands(alloc_matches),
        ("ringbuf", Some(ring_matches)) => handle_ringbuf_commands(ring_matches),
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
                .map_err(|_| renoir::error::RenoirError::invalid_parameter("size", "Invalid size format"))?;

            let backing_type = if create_matches.is_present("memfd") {
                #[cfg(target_os = "linux")]
                {
                    BackingType::MemFd
                }
                #[cfg(not(target_os = "linux"))]
                {
                    return Err(renoir::error::RenoirError::platform("memfd not supported on this platform"));
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
            println!("Created region '{}' with size {} bytes", region.name(), region.size());
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
                .map_err(|_| renoir::error::RenoirError::invalid_parameter("buffer_size", "Invalid size"))?;
            let count: usize = test_matches
                .value_of("count")
                .unwrap()
                .parse()
                .map_err(|_| renoir::error::RenoirError::invalid_parameter("count", "Invalid count"))?;

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
                    std::ptr::copy_nonoverlapping(
                        bytes.as_ptr(),
                        buffer.as_ptr() as *mut u8,
                        bytes.len(),
                    );
                }
                
                buffer_pool.return_buffer(buffer)?;
            }

            let elapsed = start.elapsed();
            let ops_per_sec = count as f64 / elapsed.as_secs_f64();

            println!("\nResults:");
            println!("  Total time: {:.2}ms", elapsed.as_millis());
            println!("  Operations/sec: {:.0}", ops_per_sec);
            println!("  Average latency: {:.2}μs", elapsed.as_micros() as f64 / count as f64);

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
                .map_err(|_| renoir::error::RenoirError::invalid_parameter("size", "Invalid size"))?;

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
            println!("  Avg time per allocation: {:.2}μs", 
                     elapsed.as_micros() as f64 / allocations.len() as f64);
        }
        ("pool", Some(pool_matches)) => {
            let size: usize = pool_matches
                .value_of("size")
                .unwrap()
                .parse()
                .map_err(|_| renoir::error::RenoirError::invalid_parameter("size", "Invalid size"))?;
            let block_size: usize = pool_matches
                .value_of("block_size")
                .unwrap()
                .parse()
                .map_err(|_| renoir::error::RenoirError::invalid_parameter("block_size", "Invalid block size"))?;

            println!("Testing pool allocator with {} bytes, block size {}", size, block_size);

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
    let capacity: usize = matches
        .value_of("capacity")
        .unwrap()
        .parse()
        .map_err(|_| renoir::error::RenoirError::invalid_parameter("capacity", "Invalid capacity"))?;
        
    let operations: usize = matches
        .value_of("operations")
        .unwrap()
        .parse()
        .map_err(|_| renoir::error::RenoirError::invalid_parameter("operations", "Invalid operations count"))?;

    if !capacity.is_power_of_two() {
        return Err(renoir::error::RenoirError::invalid_parameter(
            "capacity", 
            "Capacity must be a power of 2"
        ));
    }

    println!("Testing ring buffer with capacity {} and {} operations", capacity, operations);

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
    println!("  Push rate: {:.0} ops/sec", pushed as f64 / push_time.as_secs_f64());
    println!("  Pop rate: {:.0} ops/sec", consumed as f64 / pop_time.as_secs_f64());

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