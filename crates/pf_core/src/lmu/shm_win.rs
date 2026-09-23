//! The Windows half of the LMU shared-memory reader: the kernel32 FFI, the
//! process scan, the game's lock, and the `LMU_Data` `Reader`. **[Windows]**
//!
//! This file exists so the rest of `shm.rs` — the packed-struct transcript,
//! the layout constants, `read_struct`/`c_str`, `Snapshot` — can be built and
//! tested on any host (issue #35). Before the split, the bare
//! `#[link(name = "kernel32")]` block below made *all* of `pf_core` fail at
//! link time on Linux (`rust-lld: error: unable to find library -lkernel32`),
//! so neither the library nor its layout tests could run there. Everything
//! here is a code-motion move from `shm.rs`; the Windows build compiles the
//! same code it always did.
//!
//! Why there is no Linux reader, even though the game runs on Linux via
//! Proton: the interface publishes **win32 named kernel objects** — the
//! `LMU_Data` section, the auto-reset data event, the lock section and its
//! event — into the Wine prefix's Windows object namespace. Proton does not
//! bridge named objects into the Linux namespace, so a native Linux process
//! has nothing to open even with the game running. `Lmu::start`'s non-Windows
//! arm therefore reports [`crate::Error::LmuNotRunning`] rather than
//! pretending to a liveness it cannot observe.

use std::ffi::c_void;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::{Duration, Instant};

use crate::{Error, Result};

// The transcript items this half reads — layout constants and packed structs —
// stay in `shm.rs`, which stays portable so the golden-layout tests run on any
// host. The three `OFF_*`/size items that only `Reader::read` consumes are
// gated there and re-exported to nobody; they are visible from here because a
// child module sees its parent's private items.
use super::{
    ScoringInfoV01, Snapshot, TelemInfoV01, GAME_EXE, LAYOUT_SIZE, MAX_VEHICLES,
    OFF_ACTIVE_VEHICLES, OFF_PLAYER_IDX, OFF_SCORING, OFF_TELEMETRY, OFF_TELEM_ARRAY,
    OFF_VEH_ARRAY, SCORING_INFO_SIZE, TELEM_SIZE, VEH_SIZE,
};

type Handle = *mut c_void;
type Bool32 = i32;

const INVALID_HANDLE_VALUE: Handle = -1isize as Handle;
const FILE_MAP_READ: u32 = 0x0004;
const FILE_MAP_ALL_ACCESS: u32 = 0x000F_001F;
const PAGE_READWRITE: u32 = 0x04;
const SYNCHRONIZE: u32 = 0x0010_0000;
const WAIT_OBJECT_0: u32 = 0;
const WAIT_TIMEOUT: u32 = 0x0000_0102;
const ERROR_ALREADY_EXISTS: u32 = 183;
const INFINITE: u32 = 0xFFFF_FFFF;
const TH32CS_SNAPPROCESS: u32 = 0x0000_0002;
const MAX_PROCESS_NAME: usize = 260;

// ponytail: raw kernel32 declarations rather than the `windows` crate — this is
// the entire Win32 surface we need, and it costs no dependency.
#[link(name = "kernel32")]
extern "system" {
    fn OpenFileMappingW(access: u32, inherit: Bool32, name: *const u16) -> Handle;
    fn CreateFileMappingW(
        file: Handle,
        attrs: *mut c_void,
        protect: u32,
        size_high: u32,
        size_low: u32,
        name: *const u16,
    ) -> Handle;
    fn MapViewOfFile(
        map: Handle,
        access: u32,
        off_high: u32,
        off_low: u32,
        len: usize,
    ) -> *mut c_void;
    fn UnmapViewOfFile(addr: *const c_void) -> Bool32;
    fn OpenEventW(access: u32, inherit: Bool32, name: *const u16) -> Handle;
    fn CreateEventW(
        attrs: *mut c_void,
        manual: Bool32,
        initial: Bool32,
        name: *const u16,
    ) -> Handle;
    fn SetEvent(event: Handle) -> Bool32;
    fn WaitForSingleObject(handle: Handle, millis: u32) -> u32;
    fn WaitForMultipleObjects(count: u32, handles: *const Handle, all: Bool32, millis: u32) -> u32;
    fn OpenProcess(access: u32, inherit: Bool32, pid: u32) -> Handle;
    fn CloseHandle(handle: Handle) -> Bool32;
    fn GetLastError() -> u32;
    fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> Handle;
    fn Process32FirstW(snapshot: Handle, entry: *mut ProcessEntry32W) -> Bool32;
    fn Process32NextW(snapshot: Handle, entry: *mut ProcessEntry32W) -> Bool32;
}

#[repr(C)]
struct ProcessEntry32W {
    size: u32,
    usage: u32,
    process_id: u32,
    default_heap_id: usize,
    module_id: u32,
    threads: u32,
    parent_process_id: u32,
    pri_class_base: i32,
    flags: u32,
    exe_file: [u16; MAX_PROCESS_NAME],
}

/// A Win32 handle that closes itself. Every handle in this module is owned by
/// one of these — teardown is otherwise very easy to get wrong on the error
/// paths, and a leaked mapping handle keeps the section alive after the game
/// exits.
struct OwnedHandle(Handle);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CloseHandle(self.0) };
        }
    }
}

/// A mapped view that unmaps itself.
struct MappedView {
    addr: *mut c_void,
    len: usize,
}

impl Drop for MappedView {
    fn drop(&mut self) {
        if !self.addr.is_null() {
            unsafe { UnmapViewOfFile(self.addr) };
        }
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The PID of the running game, or `None` if it isn't up.
fn find_game_pid() -> Option<u32> {
    let snapshot = OwnedHandle(unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) });
    if snapshot.0.is_null() || snapshot.0 == INVALID_HANDLE_VALUE {
        return None;
    }
    let mut entry = ProcessEntry32W {
        size: std::mem::size_of::<ProcessEntry32W>() as u32,
        usage: 0,
        process_id: 0,
        default_heap_id: 0,
        module_id: 0,
        threads: 0,
        parent_process_id: 0,
        pri_class_base: 0,
        flags: 0,
        exe_file: [0; MAX_PROCESS_NAME],
    };
    let mut ok = unsafe { Process32FirstW(snapshot.0, &mut entry) };
    while ok != 0 {
        let end = entry.exe_file.iter().position(|&c| c == 0).unwrap_or(0);
        let name = String::from_utf16_lossy(&entry.exe_file[..end]);
        if name.eq_ignore_ascii_case(GAME_EXE) {
            return Some(entry.process_id);
        }
        ok = unsafe { Process32NextW(snapshot.0, &mut entry) };
    }
    None
}

const MAP_NAME: &str = "LMU_Data";
const DATA_EVENT_NAME: &str = "LMU_Data_Event";
const LOCK_MAP_NAME: &str = "LMU_SharedMemoryLockData";
const LOCK_EVENT_NAME: &str = "LMU_SharedMemoryLockEvent";

/// What a wait on the game returned.
pub enum Wake {
    /// The game published an update.
    Event,
    /// The wait expired without an update — the sim is paused, in the menus,
    /// or wedged. Counted as a gap.
    Timeout,
    /// The game process exited.
    GameExited,
}

/// The `LMU_Data` reader: handles, the lock, and the preallocated destination
/// buffers. One of these lives on the shared-memory thread.
pub struct Reader {
    _map: OwnedHandle,
    view: MappedView,
    data_event: OwnedHandle,
    process: Option<OwnedHandle>,
    lock: SharedMemoryLock,
    snapshot: Snapshot,
}

// SAFETY: the handles and the mapped view are process-wide kernel objects, and
// `Reader` owns its destination buffers outright. It is moved onto the reader
// thread at startup and never shared, so `Send` is all that is needed.
unsafe impl Send for Reader {}

impl Reader {
    /// Open the mapping, or say precisely why we cannot.
    ///
    /// This is the one place that decides what "LMU is up" means, and the
    /// distinction it draws is the whole reason the error is split in two: a
    /// missing process is [`Error::LmuNotRunning`], while a running process
    /// with no mapping is [`Error::LmuPluginsDisabled`] — which no amount of
    /// retrying will fix, because it needs a settings change and a restart.
    pub fn open() -> Result<Self> {
        let pid = find_game_pid().ok_or(Error::LmuNotRunning)?;

        // Open-only, never create: if the section does not exist, the game is
        // not publishing, and creating it ourselves would fake a live link.
        let map =
            OwnedHandle(unsafe { OpenFileMappingW(FILE_MAP_READ, 0, wide(MAP_NAME).as_ptr()) });
        if map.0.is_null() {
            return Err(Error::LmuPluginsDisabled);
        }
        let addr = unsafe { MapViewOfFile(map.0, FILE_MAP_READ, 0, 0, LAYOUT_SIZE) };
        if addr.is_null() {
            return Err(Error::Api(format!(
                "LMU_Data mapping could not be viewed (win32 error {})",
                unsafe { GetLastError() }
            )));
        }
        let view = MappedView {
            addr,
            len: LAYOUT_SIZE,
        };

        let data_event =
            OwnedHandle(unsafe { OpenEventW(SYNCHRONIZE, 0, wide(DATA_EVENT_NAME).as_ptr()) });
        if data_event.0.is_null() {
            return Err(Error::LmuPluginsDisabled);
        }

        // The process handle is what turns a crash into a clean shutdown rather
        // than an infinite wait. It is best-effort: LMU runs under EasyAntiCheat
        // and may refuse the handle, in which case the timeout path below still
        // notices the game is gone, one poll interval later.
        let process = {
            let h = OwnedHandle(unsafe { OpenProcess(SYNCHRONIZE, 0, pid) });
            if h.0.is_null() {
                log::debug!("LMU process handle unavailable; falling back to timeout liveness");
                None
            } else {
                Some(h)
            }
        };

        Ok(Reader {
            _map: map,
            view,
            data_event,
            process,
            lock: SharedMemoryLock::open()?,
            snapshot: Snapshot::new(),
        })
    }

    /// Is the game still publishing? Cheap enough for the timeout path.
    pub fn still_running() -> bool {
        find_game_pid().is_some()
    }

    /// Block until the game publishes an update, the game exits, or `timeout`
    /// elapses. No lock is taken here — this is the cheap side of the
    /// decimation, and it runs at the game's full ~100 Hz.
    pub fn wait(&self, timeout: Duration) -> Wake {
        let millis = timeout.as_millis().min(u128::from(INFINITE - 1)) as u32;
        match &self.process {
            Some(proc) => {
                let handles = [self.data_event.0, proc.0];
                match unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, millis) } {
                    x if x == WAIT_OBJECT_0 => Wake::Event,
                    x if x == WAIT_OBJECT_0 + 1 => Wake::GameExited,
                    _ => Wake::Timeout,
                }
            }
            None => match unsafe { WaitForSingleObject(self.data_event.0, millis) } {
                x if x == WAIT_OBJECT_0 => Wake::Event,
                x if x == WAIT_TIMEOUT && !Self::still_running() => Wake::GameExited,
                _ => Wake::Timeout,
            },
        }
    }

    /// Take the lock, memcpy the slices we use, release. Returns how long the
    /// lock was held.
    ///
    /// **Everything inside the critical section is a `copy_nonoverlapping` or
    /// an integer read.** No conversion, no allocation, no logging, no `Vec`
    /// growth — the lock is shared with the game's own writer, and time spent
    /// here is time the sim's physics thread can spend blocked.
    pub fn read(&mut self) -> Result<Duration> {
        // The unsafe block below is bounded by the layout constants, so the
        // view has to be at least that big. Checked here rather than assumed,
        // and outside the lock so it costs the game nothing.
        if self.view.len < LAYOUT_SIZE {
            return Err(Error::Api(format!(
                "LMU_Data is {} bytes, expected at least {LAYOUT_SIZE} — the game's struct layout has changed",
                self.view.len
            )));
        }
        let base = self.view.addr as *const u8;
        let started = Instant::now();
        if !self.lock.acquire(LOCK_TIMEOUT) {
            return Err(Error::Api(
                "timed out waiting for the LMU shared memory lock".into(),
            ));
        }

        // SAFETY: `base` is a view of at least LAYOUT_SIZE bytes (asserted by
        // the mapping call), and every read below is bounded by the layout
        // constants, which the const asserts above tie to the struct sizes.
        // `num` and `idx` are clamped before they are used as offsets.
        unsafe {
            let scoring = base.add(OFF_SCORING);
            std::ptr::copy_nonoverlapping(
                scoring,
                (&mut self.snapshot.scoring as *mut ScoringInfoV01).cast::<u8>(),
                SCORING_INFO_SIZE,
            );
            let num = (self.snapshot.scoring.m_num_vehicles.max(0) as usize).min(MAX_VEHICLES);
            std::ptr::copy_nonoverlapping(
                scoring.add(OFF_VEH_ARRAY),
                self.snapshot.vehicles.as_mut_ptr().cast::<u8>(),
                num * VEH_SIZE,
            );

            let telemetry = base.add(OFF_TELEMETRY);
            let active = *telemetry.add(OFF_ACTIVE_VEHICLES);
            let idx = (*telemetry.add(OFF_PLAYER_IDX) as usize).min(MAX_VEHICLES - 1);
            std::ptr::copy_nonoverlapping(
                telemetry.add(OFF_TELEM_ARRAY + idx * TELEM_SIZE),
                (&mut self.snapshot.telem as *mut TelemInfoV01).cast::<u8>(),
                TELEM_SIZE,
            );

            self.snapshot.num_vehicles = num;
            self.snapshot.player_idx = idx;
            self.snapshot.player_has_vehicle = active > 0;
        }

        self.lock.release();
        Ok(started.elapsed())
    }

    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }
}

/// How long to wait for the game's lock before giving up on a frame. The game
/// holds it for microseconds; anything near this means it is wedged, and
/// dropping one frame beats blocking the reader thread forever.
const LOCK_TIMEOUT: u32 = 50;

/// The game's own cross-process lock: a two-word spin lock in its own tiny
/// mapping, with an auto-reset event as the sleep path.
///
/// Transcribed from `SharedMemoryLock` in the shipped header, with one
/// deliberate deviation — see [`SharedMemoryLock::acquire`].
struct SharedMemoryLock {
    _map: OwnedHandle,
    view: MappedView,
    event: OwnedHandle,
}

impl SharedMemoryLock {
    fn open() -> Result<Self> {
        // Create-or-open, as the header's own sample does: whichever of the
        // game and a reader starts first owns the section.
        let map = OwnedHandle(unsafe {
            CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                std::ptr::null_mut(),
                PAGE_READWRITE,
                0,
                8,
                wide(LOCK_MAP_NAME).as_ptr(),
            )
        });
        if map.0.is_null() {
            return Err(Error::Api(
                "could not open the LMU shared memory lock".into(),
            ));
        }
        let existed = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        let addr = unsafe { MapViewOfFile(map.0, FILE_MAP_ALL_ACCESS, 0, 0, 8) };
        if addr.is_null() {
            return Err(Error::Api(
                "could not map the LMU shared memory lock".into(),
            ));
        }
        let view = MappedView { addr, len: 8 };
        let event = OwnedHandle(unsafe {
            CreateEventW(std::ptr::null_mut(), 0, 0, wide(LOCK_EVENT_NAME).as_ptr())
        });
        if event.0.is_null() {
            return Err(Error::Api("could not open the LMU lock event".into()));
        }
        let lock = SharedMemoryLock {
            _map: map,
            view,
            event,
        };
        if !existed {
            // We created it, so nobody holds it. (If the game were already up it
            // would have created it first, so this is the cold-start case.)
            lock.waiters().store(0, Ordering::Relaxed);
            lock.busy().store(0, Ordering::Relaxed);
        }
        Ok(lock)
    }

    // LockData { volatile LONG waiters; volatile LONG busy; }
    fn waiters(&self) -> &AtomicI32 {
        // SAFETY: the view is 8 bytes of shared memory holding two 4-byte
        // words; AtomicI32 has the same layout as LONG and the game touches
        // them only with interlocked operations.
        unsafe { &*(self.view.addr as *const AtomicI32) }
    }

    fn busy(&self) -> &AtomicI32 {
        unsafe { &*((self.view.addr as *const u8).add(4) as *const AtomicI32) }
    }

    /// Spin, then sleep on the event, exactly like the game's writer.
    ///
    /// Deliberate deviation from the shipped sample: its wait path returns
    /// `true` when the event fires *without* having won the compare-exchange,
    /// and leaves `waiters` incremented forever. Copying that would hand out
    /// the lock to two readers at once and leak a wakeup on every contention,
    /// so this loops until the CAS actually succeeds and always balances the
    /// counter.
    fn acquire(&self, timeout_ms: u32) -> bool {
        const MAX_SPINS: u32 = 4_000;
        for _ in 0..MAX_SPINS {
            if self.try_take() {
                return true;
            }
            std::hint::spin_loop();
        }
        self.waiters().fetch_add(1, Ordering::AcqRel);
        let deadline = Instant::now() + Duration::from_millis(u64::from(timeout_ms));
        let acquired = loop {
            if self.try_take() {
                break true;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break false;
            }
            unsafe { WaitForSingleObject(self.event.0, left.as_millis() as u32) };
        };
        self.waiters().fetch_sub(1, Ordering::AcqRel);
        acquired
    }

    fn try_take(&self) -> bool {
        self.busy()
            .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
    }

    fn release(&self) {
        self.busy().store(0, Ordering::Release);
        if self.waiters().load(Ordering::Acquire) > 0 {
            unsafe { SetEvent(self.event.0) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lmu::shm::read_struct;

    /// Diagnostic against a running session: dumps the raw LMU-extension float
    /// block around `mVirtualEnergy`, so a field that is always zero can be
    /// told apart from one we are reading at the wrong offset or sign.
    ///
    /// `cargo test -p pf_core --lib -- --ignored --nocapture shm_live_gap_block`
    #[test]
    #[ignore = "needs Le Mans Ultimate running in a session"]
    fn shm_live_gap_block() {
        let mut reader = Reader::open().expect("open LMU_Data");
        println!("  place  cars |  ve@776  ahead@780 behind@784 place_a@788 place_b@792");
        let mut seen = 0;
        while seen < 12 {
            if !matches!(reader.wait(Duration::from_secs(3)), Wake::Event) {
                println!("(no update event)");
                continue;
            }
            if reader.read().is_err() {
                continue;
            }
            let snap = reader.snapshot();
            let telem = snap.telem;
            // Read the neighbourhood raw: the header order after mVirtualEnergy
            // is CarAhead, CarBehind, PlaceAhead, PlaceBehind, all f32.
            let bytes = unsafe {
                std::slice::from_raw_parts(
                    (&telem as *const TelemInfoV01).cast::<u8>(),
                    std::mem::size_of::<TelemInfoV01>(),
                )
            };
            let f = |off: usize| -> f32 { read_struct::<f32>(bytes, off).unwrap_or(f32::NAN) };
            let place = snap.player_scoring().map(|p| p.m_place);
            println!(
                "  {:>5?} {:>5} | {:8.4} {:9.3} {:9.3} {:11.3} {:11.3}",
                place,
                snap.num_vehicles,
                f(776),
                f(780),
                f(784),
                f(788),
                f(792)
            );
            seen += 1;
            std::thread::sleep(Duration::from_millis(300));
        }
    }
}
