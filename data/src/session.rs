// Historical trading session boundary computation.
// Computes NY/London/Tokyo session open/close timestamps using jiff for DST-aware timezone handling.
use jiff::{civil, tz::TimeZone, Timestamp};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradingSession {
    NewYork,
    London,
    Tokyo,
}

impl TradingSession {
    pub const ALL: [TradingSession; 3] = [
        TradingSession::NewYork,
        TradingSession::London,
        TradingSession::Tokyo,
    ];

    pub fn label(self) -> &'static str {
        match self {
            TradingSession::NewYork => "NY",
            TradingSession::London => "LDN",
            TradingSession::Tokyo => "TKY",
        }
    }

    fn iana_zone(self) -> &'static str {
        match self {
            TradingSession::NewYork => "America/New_York",
            TradingSession::London => "Europe/London",
            TradingSession::Tokyo => "Asia/Tokyo",
        }
    }

    fn open_hm(self) -> (i8, i8) {
        match self {
            TradingSession::NewYork => (9, 30),
            TradingSession::London => (8, 0),
            TradingSession::Tokyo => (9, 0),
        }
    }

    fn close_hm(self) -> (i8, i8) {
        match self {
            TradingSession::NewYork => (16, 0),
            TradingSession::London => (16, 30),
            TradingSession::Tokyo => (15, 0),
        }
    }

    /// Session color as (r, g, b).
    pub fn color_rgb(self) -> (u8, u8, u8) {
        match self {
            TradingSession::NewYork => (40, 80, 120),
            TradingSession::London => (120, 40, 40),
            TradingSession::Tokyo => (120, 60, 100),
        }
    }
}

impl std::fmt::Display for TradingSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryKind {
    Open,
    Close,
}

impl std::fmt::Display for BoundaryKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BoundaryKind::Open => f.write_str("OPEN"),
            BoundaryKind::Close => f.write_str("CLOSE"),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SessionBoundary {
    pub session: TradingSession,
    pub kind: BoundaryKind,
    pub timestamp_ms: u64,
}

/// Compute all session open/close boundaries within `[start_ms, end_ms]` (UTC milliseconds).
///
/// Iterates calendar days with padding for timezone offsets, converts local session times
/// to UTC via jiff (handles DST automatically), skips weekends, and filters to the range.
pub fn compute_boundaries(start_ms: u64, end_ms: u64) -> Vec<SessionBoundary> {
    if start_ms >= end_ms {
        log::trace!("[SESSION] compute_boundaries: empty range start={start_ms} >= end={end_ms}");
        return Vec::new();
    }

    let mut boundaries = Vec::new();
    let mut skipped_weekends = 0u32;

    for session in TradingSession::ALL {
        let tz = match TimeZone::get(session.iana_zone()) {
            Ok(tz) => tz,
            Err(e) => {
                log::warn!(
                    "[SESSION] jiff timezone {} not found: {} — skipping {} session",
                    session.iana_zone(), e, session.label()
                );
                continue;
            }
        };

        // Pad by 2 days to handle timezone offsets (e.g., Tokyo +9 means the UTC
        // representation of a Tokyo morning session is the previous UTC day).
        let start_ts = Timestamp::from_millisecond(start_ms as i64)
            .unwrap_or(Timestamp::UNIX_EPOCH);
        let end_ts = Timestamp::from_millisecond(end_ms as i64)
            .unwrap_or(Timestamp::UNIX_EPOCH);

        let start_date = start_ts.to_zoned(tz.clone()).date();
        let end_date = end_ts.to_zoned(tz.clone()).date();

        // Iterate from start_date - 1 to end_date + 1 (inclusive)
        let mut date = start_date.yesterday().unwrap_or(start_date);
        let last_date = end_date.tomorrow().unwrap_or(end_date);

        let (open_h, open_m) = session.open_hm();
        let (close_h, close_m) = session.close_hm();
        let mut session_count = 0u32;

        while date <= last_date {
            // Skip weekends (Saturday=6, Sunday=7 in jiff)
            let weekday = date.weekday();
            if weekday == jiff::civil::Weekday::Saturday
                || weekday == jiff::civil::Weekday::Sunday
            {
                skipped_weekends += 1;
                date = date.tomorrow().unwrap_or(last_date);
                continue;
            }

            // Open boundary
            if let Some(ms) = civil_to_utc_ms(date, open_h, open_m, &tz)
                && ms >= start_ms && ms <= end_ms
            {
                log::trace!(
                    "[SESSION] {} OPEN  {date} {:02}:{:02} → UTC ms={ms}",
                    session.label(), open_h, open_m,
                );
                boundaries.push(SessionBoundary {
                    session,
                    kind: BoundaryKind::Open,
                    timestamp_ms: ms,
                });
                session_count += 1;
            }

            // Close boundary
            if let Some(ms) = civil_to_utc_ms(date, close_h, close_m, &tz)
                && ms >= start_ms && ms <= end_ms
            {
                log::trace!(
                    "[SESSION] {} CLOSE {date} {:02}:{:02} → UTC ms={ms}",
                    session.label(), close_h, close_m,
                );
                boundaries.push(SessionBoundary {
                    session,
                    kind: BoundaryKind::Close,
                    timestamp_ms: ms,
                });
                session_count += 1;
            }

            date = match date.tomorrow() {
                Ok(d) => d,
                Err(_) => break,
            };
        }

        log::trace!(
            "[SESSION] {} produced {session_count} boundaries (date range: {start_date} to {end_date})",
            session.label(),
        );
    }

    boundaries.sort_by_key(|b| b.timestamp_ms);

    log::debug!(
        "[SESSION] compute_boundaries: range=[{start_ms}, {end_ms}] → {} boundaries, {skipped_weekends} weekend days skipped",
        boundaries.len(),
    );

    boundaries
}

/// Convert a civil date + time in a timezone to UTC milliseconds.
fn civil_to_utc_ms(date: civil::Date, hour: i8, minute: i8, tz: &TimeZone) -> Option<u64> {
    let dt = date.at(hour, minute, 0, 0);
    let zoned = dt.to_zoned(tz.clone()).ok()?;
    let ms = zoned.timestamp().as_millisecond();
    if ms < 0 { None } else { Some(ms as u64) }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // Oracle values: independently computed via macOS `date` command
    // (uses OS libc timezone, NOT jiff). Each value cross-verified by
    // converting back to UTC and checking against known UTC offset rules.
    // ========================================================================

    // --- 2026-03-02 (Monday, US EST / UK GMT / JST) ---
    const NY_OPEN_20260302_MS: u64  = 1772461800_000; // 09:30 EST = 14:30 UTC
    const NY_CLOSE_20260302_MS: u64 = 1772485200_000; // 16:00 EST = 21:00 UTC
    const LDN_OPEN_20260302_MS: u64  = 1772438400_000; // 08:00 GMT = 08:00 UTC
    const LDN_CLOSE_20260302_MS: u64 = 1772469000_000; // 16:30 GMT = 16:30 UTC
    const TKY_OPEN_20260302_MS: u64  = 1772409600_000; // 09:00 JST = 00:00 UTC
    const TKY_CLOSE_20260302_MS: u64 = 1772431200_000; // 15:00 JST = 06:00 UTC

    // --- 2026-03-06 (Friday, last EST day before spring forward) ---
    const NY_OPEN_20260306_MS: u64  = 1772807400_000; // 09:30 EST = 14:30 UTC
    const NY_CLOSE_20260306_MS: u64 = 1772830800_000; // 16:00 EST = 21:00 UTC

    // --- 2026-03-09 (Monday, first EDT day after spring forward 2026-03-08) ---
    const NY_OPEN_20260309_MS: u64  = 1773063000_000; // 09:30 EDT = 13:30 UTC
    const NY_CLOSE_20260309_MS: u64 = 1773086400_000; // 16:00 EDT = 20:00 UTC

    // --- 2026-03-30 (Monday, first BST day in UK, spring forward 2026-03-29) ---
    const LDN_OPEN_20260330_MS: u64 = 1774854000_000; // 08:00 BST = 07:00 UTC

    // ========================================================================
    // Level 1: Atomic — civil_to_utc_ms primitive operation
    // ========================================================================

    #[test]
    fn atomic_civil_to_utc_ny_est() {
        let tz = TimeZone::get("America/New_York").unwrap();
        let date = civil::date(2026, 3, 2);
        let ms = civil_to_utc_ms(date, 9, 30, &tz).unwrap();
        assert_eq!(ms, NY_OPEN_20260302_MS, "NY open 2026-03-02 EST");
    }

    #[test]
    fn atomic_civil_to_utc_ny_edt() {
        let tz = TimeZone::get("America/New_York").unwrap();
        let date = civil::date(2026, 3, 9);
        let ms = civil_to_utc_ms(date, 9, 30, &tz).unwrap();
        assert_eq!(ms, NY_OPEN_20260309_MS, "NY open 2026-03-09 EDT");
    }

    #[test]
    fn atomic_civil_to_utc_london_gmt() {
        let tz = TimeZone::get("Europe/London").unwrap();
        let date = civil::date(2026, 3, 2);
        let ms = civil_to_utc_ms(date, 8, 0, &tz).unwrap();
        assert_eq!(ms, LDN_OPEN_20260302_MS, "LDN open 2026-03-02 GMT");
    }

    #[test]
    fn atomic_civil_to_utc_london_bst() {
        let tz = TimeZone::get("Europe/London").unwrap();
        let date = civil::date(2026, 3, 30);
        let ms = civil_to_utc_ms(date, 8, 0, &tz).unwrap();
        assert_eq!(ms, LDN_OPEN_20260330_MS, "LDN open 2026-03-30 BST");
    }

    #[test]
    fn atomic_civil_to_utc_tokyo() {
        let tz = TimeZone::get("Asia/Tokyo").unwrap();
        let date = civil::date(2026, 3, 2);
        let ms_open = civil_to_utc_ms(date, 9, 0, &tz).unwrap();
        let ms_close = civil_to_utc_ms(date, 15, 0, &tz).unwrap();
        assert_eq!(ms_open, TKY_OPEN_20260302_MS, "TKY open 2026-03-02 JST");
        assert_eq!(ms_close, TKY_CLOSE_20260302_MS, "TKY close 2026-03-02 JST");
    }

    // ========================================================================
    // Level 2: Composition — compute_boundaries for a single day
    // ========================================================================

    /// Extract a specific boundary from the result by (session, kind).
    fn find_boundary(
        boundaries: &[SessionBoundary],
        session: TradingSession,
        kind: BoundaryKind,
    ) -> Option<u64> {
        boundaries
            .iter()
            .find(|b| b.session == session && b.kind == kind)
            .map(|b| b.timestamp_ms)
    }

    #[test]
    fn single_day_all_sessions_20260302() {
        // Range that covers the entire Monday 2026-03-02 in UTC
        // Tokyo opens at 00:00 UTC, NY closes at 21:00 UTC
        let start = TKY_OPEN_20260302_MS - 1000; // just before Tokyo open
        let end = NY_CLOSE_20260302_MS + 1000;     // just after NY close

        let boundaries = compute_boundaries(start, end);

        // Exactly 6 boundaries: 3 sessions × (open + close)
        assert_eq!(
            boundaries.len(), 6,
            "expected 6 boundaries for single weekday, got {}: {:?}",
            boundaries.len(),
            boundaries.iter().map(|b| (b.session.label(), format!("{}", b.kind), b.timestamp_ms)).collect::<Vec<_>>()
        );

        // Bit-exact verification against oracle
        assert_eq!(
            find_boundary(&boundaries, TradingSession::NewYork, BoundaryKind::Open),
            Some(NY_OPEN_20260302_MS),
            "NY OPEN"
        );
        assert_eq!(
            find_boundary(&boundaries, TradingSession::NewYork, BoundaryKind::Close),
            Some(NY_CLOSE_20260302_MS),
            "NY CLOSE"
        );
        assert_eq!(
            find_boundary(&boundaries, TradingSession::London, BoundaryKind::Open),
            Some(LDN_OPEN_20260302_MS),
            "LDN OPEN"
        );
        assert_eq!(
            find_boundary(&boundaries, TradingSession::London, BoundaryKind::Close),
            Some(LDN_CLOSE_20260302_MS),
            "LDN CLOSE"
        );
        assert_eq!(
            find_boundary(&boundaries, TradingSession::Tokyo, BoundaryKind::Open),
            Some(TKY_OPEN_20260302_MS),
            "TKY OPEN"
        );
        assert_eq!(
            find_boundary(&boundaries, TradingSession::Tokyo, BoundaryKind::Close),
            Some(TKY_CLOSE_20260302_MS),
            "TKY CLOSE"
        );

        // Verify sorted order invariant (chronological)
        for w in boundaries.windows(2) {
            assert!(w[0].timestamp_ms <= w[1].timestamp_ms, "not sorted: {} > {}", w[0].timestamp_ms, w[1].timestamp_ms);
        }

        // Verify chronological ordering matches known UTC sequence:
        // TKY open (00:00) < TKY close (06:00) < LDN open (08:00) < NY open (14:30)
        // < LDN close (16:30) < NY close (21:00)
        let ts: Vec<u64> = boundaries.iter().map(|b| b.timestamp_ms).collect();
        assert_eq!(ts, vec![
            TKY_OPEN_20260302_MS,
            TKY_CLOSE_20260302_MS,
            LDN_OPEN_20260302_MS,
            NY_OPEN_20260302_MS,
            LDN_CLOSE_20260302_MS,
            NY_CLOSE_20260302_MS,
        ], "chronological order mismatch");
    }

    // ========================================================================
    // Level 3: DST transition — the hardest boundary condition
    // ========================================================================

    #[test]
    fn dst_transition_ny_spring_forward() {
        // Range covering Friday (EST) through Monday (EDT)
        let start = NY_OPEN_20260306_MS - 1000;
        let end = NY_CLOSE_20260309_MS + 1000;

        let boundaries = compute_boundaries(start, end);

        // Find NY boundaries only
        let ny_bounds: Vec<_> = boundaries
            .iter()
            .filter(|b| b.session == TradingSession::NewYork)
            .collect();

        // Friday (EST): open=14:30 UTC, close=21:00 UTC
        assert_eq!(ny_bounds[0].timestamp_ms, NY_OPEN_20260306_MS, "Fri NY OPEN (EST)");
        assert_eq!(ny_bounds[0].kind, BoundaryKind::Open);
        assert_eq!(ny_bounds[1].timestamp_ms, NY_CLOSE_20260306_MS, "Fri NY CLOSE (EST)");
        assert_eq!(ny_bounds[1].kind, BoundaryKind::Close);

        // Monday (EDT): open=13:30 UTC, close=20:00 UTC (1 hour earlier!)
        assert_eq!(ny_bounds[2].timestamp_ms, NY_OPEN_20260309_MS, "Mon NY OPEN (EDT)");
        assert_eq!(ny_bounds[2].kind, BoundaryKind::Open);
        assert_eq!(ny_bounds[3].timestamp_ms, NY_CLOSE_20260309_MS, "Mon NY CLOSE (EDT)");
        assert_eq!(ny_bounds[3].kind, BoundaryKind::Close);

        // DST shift: Monday open is 1 hour earlier in UTC than Friday open
        // (both are 09:30 local, but EST=-5 vs EDT=-4)
        let fri_open_utc_hour = (NY_OPEN_20260306_MS / 1000 % 86400) / 3600;
        let mon_open_utc_hour = (NY_OPEN_20260309_MS / 1000 % 86400) / 3600;
        assert_eq!(fri_open_utc_hour, 14, "Fri open = 14:30 UTC");
        assert_eq!(mon_open_utc_hour, 13, "Mon open = 13:30 UTC");
    }

    // ========================================================================
    // Level 4: Weekend exclusion invariant
    // ========================================================================

    #[test]
    fn weekends_are_excluded() {
        // 2026-03-07 = Saturday, 2026-03-08 = Sunday
        // Range: Saturday 00:00 UTC to Sunday 23:59 UTC
        let sat_start = 1772870400_000u64; // 2026-03-07 00:00 UTC
        let sun_end = 1772956799_000u64;   // 2026-03-08 23:59:59 UTC

        let boundaries = compute_boundaries(sat_start, sun_end);

        // No sessions should have boundaries on Saturday/Sunday
        assert!(
            boundaries.is_empty(),
            "expected no boundaries on weekend, got {}: {:?}",
            boundaries.len(),
            boundaries.iter().map(|b| (b.session.label(), format!("{}", b.kind), b.timestamp_ms)).collect::<Vec<_>>()
        );
    }

    // ========================================================================
    // Level 5: Empty and degenerate inputs
    // ========================================================================

    #[test]
    fn empty_range() {
        let b = compute_boundaries(1000, 1000);
        assert!(b.is_empty(), "equal start/end should return empty");
    }

    #[test]
    fn inverted_range() {
        let b = compute_boundaries(2000, 1000);
        assert!(b.is_empty(), "inverted range should return empty");
    }

    #[test]
    fn narrow_range_misses_all_sessions() {
        // A 1-second window in the middle of the night (02:00 UTC on a weekday)
        // No session is open/closing at this moment
        let base = 1772416800_000u64; // 2026-03-02 02:00:00 UTC
        let b = compute_boundaries(base, base + 1000);
        assert!(b.is_empty(), "1-second window at 02:00 UTC should have no boundaries");
    }

    // ========================================================================
    // Level 6: Multi-day span — count invariant
    // ========================================================================

    #[test]
    fn five_weekday_span_boundary_count() {
        // Mon 2026-03-02 to Fri 2026-03-06, covering all sessions
        // Use a wide range to capture everything
        let start = TKY_OPEN_20260302_MS - 1; // just before Mon Tokyo open
        let end = NY_CLOSE_20260306_MS + 1;     // just after Fri NY close

        let boundaries = compute_boundaries(start, end);

        // 5 weekdays × 3 sessions × 2 (open+close) = 30 boundaries
        let ny_count = boundaries.iter().filter(|b| b.session == TradingSession::NewYork).count();
        let ldn_count = boundaries.iter().filter(|b| b.session == TradingSession::London).count();
        let tky_count = boundaries.iter().filter(|b| b.session == TradingSession::Tokyo).count();

        assert_eq!(ny_count, 10, "NY: 5 days × 2 = 10");
        assert_eq!(ldn_count, 10, "LDN: 5 days × 2 = 10");
        assert_eq!(tky_count, 10, "TKY: 5 days × 2 = 10");
        assert_eq!(boundaries.len(), 30, "total: 5 days × 3 sessions × 2 = 30");

        // Every open must be paired with a close for the same session
        for session in TradingSession::ALL {
            let opens: Vec<u64> = boundaries.iter()
                .filter(|b| b.session == session && b.kind == BoundaryKind::Open)
                .map(|b| b.timestamp_ms)
                .collect();
            let closes: Vec<u64> = boundaries.iter()
                .filter(|b| b.session == session && b.kind == BoundaryKind::Close)
                .map(|b| b.timestamp_ms)
                .collect();
            assert_eq!(opens.len(), closes.len(), "{}: open/close count mismatch", session.label());
            // Each open must precede its corresponding close
            for (o, c) in opens.iter().zip(closes.iter()) {
                assert!(o < c, "{}: open {} not before close {}", session.label(), o, c);
            }
        }
    }

    // ========================================================================
    // Level 7: Parameter variation — Tokyo has no DST
    // ========================================================================

    #[test]
    fn tokyo_utc_offset_is_constant() {
        // Tokyo is JST (UTC+9) year-round. Verify the UTC offset doesn't change
        // across the US/UK DST transition dates.

        // Before any DST (2026-01-05, Monday)
        let tz = TimeZone::get("Asia/Tokyo").unwrap();
        let jan = civil_to_utc_ms(civil::date(2026, 1, 5), 9, 0, &tz).unwrap();

        // During US DST but before UK DST (2026-03-09, Monday)
        let mar = civil_to_utc_ms(civil::date(2026, 3, 9), 9, 0, &tz).unwrap();

        // During both US and UK DST (2026-06-01, Monday)
        let jun = civil_to_utc_ms(civil::date(2026, 6, 1), 9, 0, &tz).unwrap();

        // All should have the same time-of-day in UTC (00:00)
        assert_eq!(jan % 86_400_000, 0, "Jan TKY open should be 00:00 UTC");
        assert_eq!(mar % 86_400_000, 0, "Mar TKY open should be 00:00 UTC");
        assert_eq!(jun % 86_400_000, 0, "Jun TKY open should be 00:00 UTC");
    }
}
