use std::collections::VecDeque;
use std::ffi::OsStr;

use byte_unit::Byte;
use chrono::{DateTime, Local, Utc};
use itertools::Itertools as _;
use num_traits::Zero;
use ordered_float::OrderedFloat;
use serde::Serialize;
use sysinfo::ProcessesToUpdate;
use warp_core::channel::ChannelState;
use warpui::{App, AppContext, Entity, ModelContext, SingletonEntity};

use crate::server::telemetry;
use crate::system::memory_footprint;
use crate::terminal::TerminalView;
use crate::{send_telemetry_from_app_ctx, send_telemetry_sync_from_ctx, TelemetryEvent};

/// The threshold at which we emit a memory usage warning.
const MEMORY_USAGE_WARNING_THRESHOLD: Option<Byte> = byte_unit::Byte::GIGABYTE.multiply(10);

/// The refresh interval for system information, in seconds.
const REFRESH_INTERVAL_S: usize = 5;
/// The refresh interval for system information.
const REFRESH_INTERVAL: std::time::Duration =
    std::time::Duration::from_secs(REFRESH_INTERVAL_S as u64);

/// The time window that a resource usage report covers, in seconds.
const REPORT_WINDOW_S: usize = 300;
/// The number of data points aggregated into a resource usage report.
const REPORT_SAMPLE_COUNT: usize = REPORT_WINDOW_S / REFRESH_INTERVAL_S;

// Make sure the refresh interval cleanly divides the report window into an
// integral number of samples.
static_assertions::const_assert_eq!(REPORT_WINDOW_S % REFRESH_INTERVAL_S, 0);

pub enum SystemInfoEvent {
    /// There is new system info available for consumers to query.
    Refreshed,
    /// The application is using a large quantity of memory.
    MemoryUsageHigh,
}

pub struct SystemInfo {
    /// A structure we can use to efficiently query system information.
    system: sysinfo::System,
    /// Whether or not we've already emitted an event due to high memory usage.
    has_emitted_memory_warning_event: bool,
    /// A circular buffer storing resource usage data.
    stats: StatsBuffer,
    /// A helper structure for reporting resource usage via telemetry events.
    resource_usage_reporter: ResourceUsageReporter,
    /// The long OS version.
    long_os_version: Option<String>,
}

impl SystemInfo {
    /// Creates a new [`SystemInfo`] model and begins periodic fetching of
    /// system information.
    ///
    /// Currently only retrieves and exposes memory usage information for the
    /// current process.
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let mut me = Self {
            system: sysinfo::System::new(),
            has_emitted_memory_warning_event: false,
            stats: Default::default(),
            resource_usage_reporter: Default::default(),
            long_os_version: sysinfo::System::long_os_version(),
        };

        // Initialize the underlying system info.  This is necessary in order
        // for our first read of CPU stats to be accurate, as they are computed
        // as a delta between the previous refresh and now.
        me.system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[Self::current_pid()]),
            false, /* refresh_dead_processes */
            Self::refresh_kind(),
        );

        // If we're doing automated heap usage tracking, set up periodic
        // refreshes of the memory usage data.
        Self::schedule_refresh(ctx);

        me
    }

    pub fn handle_block_created(&mut self) {
        self.resource_usage_reporter.handle_block_created();
    }

    /// Returns the amount of memory being used by the current process, in
    /// bytes.
    pub fn used_memory(&self) -> Byte {
        self.system
            .process(Self::current_pid())
            .expect("current process should exist")
            .memory()
            .into()
    }

    /// Returns the full memory footprint of the current process, in bytes.
    ///
    /// Unlike [`used_memory`] (RSS), this includes memory that has been
    /// swapped out or compressed by the OS.  On macOS this matches the value
    /// shown by Activity Monitor.
    pub fn memory_footprint(&self) -> Byte {
        memory_footprint::memory_footprint_bytes().into()
    }

    /// Returns the average CPU usage over the refresh interval.
    ///
    /// If one CPU core is utilized at 100%, this will return 1.  It may return
    /// a value >1 on multi-core machines.
    pub fn cpu_usage(&self) -> f32 {
        let total_usage = self
            .system
            .process(Self::current_pid())
            .expect("current process should exist")
            .cpu_usage();
        total_usage / 100.
    }

    pub fn long_os_version(&self) -> Option<&str> {
        self.long_os_version.as_deref()
    }

    fn schedule_refresh(ctx: &mut ModelContext<Self>) {
        ctx.spawn(
            async {
                warpui::r#async::Timer::after(REFRESH_INTERVAL).await;
            },
            |me, _, ctx| {
                me.refresh(ctx);
                Self::schedule_refresh(ctx);
            },
        );
    }

    fn refresh(&mut self, ctx: &mut ModelContext<Self>) {
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[Self::current_pid()]),
            false, /* refresh_dead_processes */
            Self::refresh_kind(),
        );
        ctx.emit(SystemInfoEvent::Refreshed);

        // Add resource usage information to our circular buffer.
        self.stats.push(Sample {
            cpu: self.cpu_usage(),
        });

        let rss = self.used_memory();
        let footprint = self.memory_footprint();
        self.check_for_excessive_memory_usage(rss, footprint, ctx);

        // Once we have a full buffer of statistics, consider sending a report
        // each time we store new resource usage data.
        if self.stats.is_full() {
            self.resource_usage_reporter.maybe_send_report(ctx);
        }
    }

    /// Checks for excessive memory usage.  This may send a telemetry event
    /// and trigger a Sentry heap profile dump if excessive usage is detected.
    ///
    /// The threshold check uses `memory_footprint` (which includes swapped
    /// and compressed pages) so we actually detect high memory situations.
    /// The Rudderstack telemetry event still reports `rss` so existing
    /// dashboards are unaffected.
    fn check_for_excessive_memory_usage(
        &mut self,
        rss: Byte,
        memory_footprint: Byte,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.has_emitted_memory_warning_event {
            return;
        }

        // Use footprint (not RSS) for the threshold so we catch memory
        // that has been swapped out or compressed by the OS.
        if memory_footprint
            < MEMORY_USAGE_WARNING_THRESHOLD.expect("Threshold should not overflow u64")
        {
            return;
        }

        // Collect a detailed memory breakdown for diagnostics.
        let memory_breakdown = memory_footprint::memory_breakdown();

        // If we're tracking heap usage and detect excessive memory usage,
        // dump and upload the current heap profiling data.
        #[cfg(feature = "heap_usage_tracking")]
        {
            let breakdown_for_sentry = memory_breakdown.clone();
            ctx.spawn(
                crate::profiling::dump_jemalloc_heap_profile(breakdown_for_sentry),
                |_, _, _| {},
            );
        }

        // Send a telemetry event indicating that memory usage is extreme.
        // Report RSS here to keep Rudderstack dashboards consistent.
        let total_application_usage_bytes = rss.as_u64();
        send_telemetry_sync_from_ctx!(
            TelemetryEvent::MemoryUsageHigh {
                total_application_usage_bytes,
                memory_breakdown,
            },
            ctx
        );

        ctx.emit(SystemInfoEvent::MemoryUsageHigh);
        self.has_emitted_memory_warning_event = true;
    }

    /// Returns the pid of the current process.
    fn current_pid() -> sysinfo::Pid {
        sysinfo::get_current_pid().expect("Platform should support process IDs")
    }

    /// Returns the [`sysinfo::ProcessRefreshKind`] that should be used when
    /// retrieving information about the current process.
    fn refresh_kind() -> sysinfo::ProcessRefreshKind {
        sysinfo::ProcessRefreshKind::nothing()
            .with_memory()
            .with_cpu()
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn refresh_all_processes(&mut self) {
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true, /* remove_dead_processes */
            Self::refresh_kind(),
        );
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn processes_by_name<'a>(
        &'a self,
        name: &'a str,
    ) -> impl Iterator<Item = &'a sysinfo::Process> {
        self.system.processes_by_name(OsStr::new(name))
    }
}

impl Entity for SystemInfo {
    type Event = SystemInfoEvent;
}

impl SingletonEntity for SystemInfo {}

/// Helper structure for making resource usage reports.
struct ResourceUsageReporter {
    /// The number of blocks created since we last reported on resource usage
    /// statistics.
    blocks_created_since_last_report: usize,

    /// The time at which we sent the last report.
    time_last_report_sent: DateTime<Utc>,
}

impl ResourceUsageReporter {
    /// We won't produce a new report unless the user has created at least
    /// this many blocks since the last one.
    const MIN_BLOCKS_CREATED_PER_MEMORY_REPORT: usize = 5;
    /// We won't produce a new report unless at least this much time has
    /// passed since the last one.
    const MIN_DURATION_BETWEEN_MEMORY_REPORTS: chrono::Duration = chrono::Duration::hours(1);
    /// We won't produce a report unless the user has been active recently.
    const USER_RECENTLY_ACTIVE_INTERVAL: chrono::Duration = chrono::Duration::minutes(5);

    /// Handles creation of a block in a blocklist.
    fn handle_block_created(&mut self) {
        self.blocks_created_since_last_report += 1;
    }

    /// Sends a resource usage report if the required conditions are met.
    fn maybe_send_report(&mut self, ctx: &mut ModelContext<SystemInfo>) {
        if self.should_send_report() {
            // Immediately set the time at which we sent the last report, to
            // ensure we don't send two if it takes a little while to schedule
            // the background task below.
            self.time_last_report_sent = Utc::now();

            // We do this in a task callback to ensure that all terminal views
            // will be returned when iterating over the app context.  Without
            // this, we'll skip the active terminal view, as it has been
            // removed from the app context temporarily in order to provide
            // mutable access to it.
            ctx.spawn(futures::future::ready(()), |me, _, ctx| {
                me.refresh(ctx);
                let total_application_usage = me.used_memory();
                me.resource_usage_reporter.send_report(
                    total_application_usage,
                    me.stats.iter(),
                    ctx,
                );
            });
        }
    }

    /// Returns whether or not it's time to generate a report.
    fn should_send_report(&self) -> bool {
        // Don't send reports too frequently.
        if Utc::now().signed_duration_since(self.time_last_report_sent)
            < Self::MIN_DURATION_BETWEEN_MEMORY_REPORTS
        {
            return false;
        }

        // If we don't know when the user was last active, don't send a report.
        let Some(last_active_time) =
            DateTime::<Utc>::from_timestamp(App::last_active_timestamp(), 0)
        else {
            return false;
        };

        // Don't send a report unless the user has been active recently.
        if Utc::now().signed_duration_since(last_active_time) > Self::USER_RECENTLY_ACTIVE_INTERVAL
        {
            return false;
        }

        true
    }

    /// Sends a resource usage report.
    fn send_report<'a>(
        &mut self,
        total_application_usage: Byte,
        samples: impl Iterator<Item = &'a Sample>,
        ctx: &mut AppContext,
    ) {
        let _cpu_usage_stats = Self::compute_cpu_usage_stats(samples);
        let memory_usage_stats = Self::compute_memory_usage_stats(total_application_usage, ctx);

        // We send two different events at the moment, as one contains general
        // resource usage information, and one contains more detailed info
        // about memory consumption caused by the blocklist.
        //
        // TODO(vorporeal): Clean up the memory usage one, either eliminating it
        // or merging it into the general resource usage telemetry event.

        // Only send detailed memory usage reports in dogfood, for the time being.
        if ChannelState::channel().is_dogfood() {
            // Only send the detailed memory usage report if the user has created
            // enough blocks since the last detailed memory usage report.
            if self.blocks_created_since_last_report >= Self::MIN_BLOCKS_CREATED_PER_MEMORY_REPORT {
                send_telemetry_from_app_ctx!(TelemetryEvent::from(memory_usage_stats), ctx);
                self.blocks_created_since_last_report = 0;
            }
        }
    }

    fn compute_cpu_usage_stats<'a>(samples: impl Iterator<Item = &'a Sample>) -> CpuUsageStats {
        let mut num_samples = 0;
        let mut avg_usage = 0.;
        let mut max_usage = OrderedFloat::zero();
        for sample in samples {
            num_samples += 1;
            avg_usage += sample.cpu;
            max_usage = std::cmp::max(max_usage, sample.cpu.into());
        }

        avg_usage /= num_samples as f32;

        let num_cpus = num_cpus::get();
        CpuUsageStats {
            num_cpus,
            avg_usage,
            max_usage: max_usage.into_inner(),
        }
    }

    fn compute_memory_usage_stats(
        total_application_usage: Byte,
        ctx: &mut AppContext,
    ) -> MemoryUsageStats {
        let mut stats = MemoryUsageStats::new(total_application_usage);

        // Don't compute detailed memory usage statistics outside of debug builds.
        if !ChannelState::enable_debug_features() {
            return stats;
        }

        let now = Local::now();

        // Loop over all terminal views, collecting information about how
        // many blocks they contain, number of lines, amount of memory,
        // and the active/inactive breakdown.
        for window_id in ctx.window_ids().collect_vec() {
            for terminal_view in ctx
                .views_of_type::<TerminalView>(window_id)
                .into_iter()
                .flatten()
                .map(|handle| handle.as_ref(ctx))
            {
                let model = terminal_view.model.lock();
                stats.add_blocks(now, model.block_list().blocks().iter());
            }
        }

        stats
    }
}

impl Default for ResourceUsageReporter {
    fn default() -> Self {
        Self {
            blocks_created_since_last_report: 0,
            time_last_report_sent: DateTime::UNIX_EPOCH,
        }
    }
}

/// Statistics about CPU usage.
struct CpuUsageStats {
    /// The number of "CPUs" on the machine.  This actually measure the number
    /// of _logical_ CPUs, i.e.: CPU cores (including SMT pseudo-cores).
    num_cpus: usize,
    /// The maximum CPU usage over the measurement interval, represented as a
    /// value in the range [0, num_cpus].
    max_usage: f32,
    /// The average CPU usage over the measurement interval, represented as a
    /// value in the range [0, num_cpus].
    avg_usage: f32,
}

impl From<CpuUsageStats> for telemetry::CpuUsageStats {
    fn from(value: CpuUsageStats) -> Self {
        Self {
            num_cpus: value.num_cpus,
            max_usage: value.max_usage,
            avg_usage: value.avg_usage,
        }
    }
}

#[derive(Copy, Clone)]
struct MemoryUsageStats {
    total_application_usage_bytes: usize,
    total_blocks: usize,
    total_lines: usize,

    /// Statistics about blocks that have been seen in the past 5 minutes.
    active_block_stats: BlockMemoryStats,
    /// Statistics about blocks that haven't been seen since [5m, 1h).
    inactive_5m_stats: BlockMemoryStats,
    /// Statistics about blocks that haven't been seen since [1h, 24h).
    inactive_1h_stats: BlockMemoryStats,
    /// Statistics about blocks that haven't been seen since [24h, ..).
    inactive_24h_stats: BlockMemoryStats,
}

impl MemoryUsageStats {
    fn new(total_application_usage: Byte) -> Self {
        Self {
            total_application_usage_bytes: total_application_usage.as_u64() as usize,
            total_blocks: 0,
            total_lines: 0,
            active_block_stats: Default::default(),
            inactive_5m_stats: Default::default(),
            inactive_1h_stats: Default::default(),
            inactive_24h_stats: Default::default(),
        }
    }

    fn add_blocks<'a>(
        &mut self,
        now: DateTime<Local>,
        blocks: impl Iterator<Item = &'a crate::terminal::model::block::Block>,
    ) {
        // We compute block-related memory stats across various intervals.
        // "Activity" refers to how recently the block was painted.
        const DURATION_5M: chrono::Duration = chrono::Duration::minutes(5);
        const DURATION_1H: chrono::Duration = chrono::Duration::hours(1);
        const DURATION_24H: chrono::Duration = chrono::Duration::hours(24);

        for block in blocks {
            let num_lines: usize = block.all_grids_iter().map(|grid| grid.len()).sum();

            self.total_blocks += 1;
            self.total_lines += num_lines;

            let last_painted_at = block
                .last_painted_at()
                .unwrap_or(DateTime::UNIX_EPOCH.into());
            let stats = match now - last_painted_at {
                duration if duration < DURATION_5M => &mut self.active_block_stats,
                duration if duration < DURATION_1H => &mut self.inactive_5m_stats,
                duration if duration < DURATION_24H => &mut self.inactive_1h_stats,
                _ => &mut self.inactive_24h_stats,
            };

            stats.num_blocks += 1;
            stats.num_lines += num_lines;
            stats.estimated_memory_usage_bytes += block.estimated_memory_usage_bytes();
        }
    }
}

impl From<MemoryUsageStats> for TelemetryEvent {
    fn from(value: MemoryUsageStats) -> Self {
        TelemetryEvent::MemoryUsageStats {
            total_application_usage_bytes: value.total_application_usage_bytes,
            total_blocks: value.total_blocks,
            total_lines: value.total_lines,
            active_block_stats: value.active_block_stats.into(),
            inactive_5m_stats: value.inactive_5m_stats.into(),
            inactive_1h_stats: value.inactive_1h_stats.into(),
            inactive_24h_stats: value.inactive_24h_stats.into(),
        }
    }
}

impl From<MemoryUsageStats> for telemetry::MemoryUsageStats {
    fn from(value: MemoryUsageStats) -> Self {
        Self {
            total_application_usage_bytes: value.total_application_usage_bytes,
            total_blocks: value.total_blocks,
            total_lines: value.total_lines,
            active_block_stats: value.active_block_stats.into(),
            inactive_5m_stats: value.inactive_5m_stats.into(),
            inactive_1h_stats: value.inactive_1h_stats.into(),
            inactive_24h_stats: value.inactive_24h_stats.into(),
        }
    }
}

#[derive(Copy, Clone, Default, Serialize, PartialEq)]
struct BlockMemoryStats {
    num_blocks: usize,
    num_lines: usize,
    estimated_memory_usage_bytes: usize,
}

impl std::fmt::Debug for BlockMemoryStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlockMemoryStats")
            .field("num_blocks", &self.num_blocks)
            .field("num_lines", &self.num_lines)
            .field(
                "estimated_memory_usage_bytes",
                &byte_unit::Byte::from(self.estimated_memory_usage_bytes)
                    .get_adjusted_unit(byte_unit::Unit::MB),
            )
            .finish()
    }
}

impl From<BlockMemoryStats> for telemetry::BlockMemoryUsageStats {
    fn from(value: BlockMemoryStats) -> Self {
        Self {
            num_blocks: value.num_blocks,
            num_lines: value.num_lines,
            estimated_memory_usage_bytes: value.estimated_memory_usage_bytes,
        }
    }
}

/// A single resource usage sample point.
struct Sample {
    /// The CPU usage since the last sample, represented as a value in the
    /// range [0, num_cpus].
    cpu: f32,
}

/// A simple fixed-size circular buffer for storing resource usage sample
/// points.
struct StatsBuffer {
    stats: VecDeque<Sample>,
}

impl StatsBuffer {
    /// Constructs a new [`StatsBuffer`].
    fn new() -> Self {
        Self {
            stats: VecDeque::with_capacity(REPORT_SAMPLE_COUNT),
        }
    }

    /// Returns whether or not the buffer is full of samples.
    ///
    /// If true, adding a sample will replace the oldest sample in the buffer.
    fn is_full(&self) -> bool {
        self.stats.len() == self.stats.capacity()
    }

    /// Pushes a new sample into the buffer.  If the buffer is at capacity,
    /// the oldest sample will be removed to make room for the new one.
    fn push(&mut self, sample: Sample) {
        if self.is_full() {
            self.stats.pop_front();
        }
        self.stats.push_back(sample);
    }

    /// Returns an iterator over all samples in the buffer.
    fn iter(&self) -> impl Iterator<Item = &Sample> {
        self.stats.iter()
    }
}

impl Default for StatsBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "info_tests.rs"]
mod tests;
