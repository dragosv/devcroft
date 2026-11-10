// Diagnostic observer only; no task ports, code injection or target mutation.
// CPU units are calibrated against POSIX process CPU time before sampling.
#include <errno.h>
#include <inttypes.h>
#include <libproc.h>
#include <limits.h>
#include <mach/mach_time.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/resource.h>
#include <time.h>
#include <unistd.h>

static uint32_t scale_num = 1, scale_den = 1;

static uint64_t process_cpu_ns(void) {
    struct timespec t;
    if (clock_gettime(CLOCK_PROCESS_CPUTIME_ID, &t)) exit(2);
    return (uint64_t)t.tv_sec * 1000000000 + (uint64_t)t.tv_nsec;
}

static int usage(pid_t pid, struct rusage_info_v4 *info) {
    memset(info, 0, sizeof(*info));
    return proc_pid_rusage(pid, RUSAGE_INFO_V4, (rusage_info_t *)info);
}

static uint64_t nanos(uint64_t raw) {
    return (uint64_t)(((__uint128_t)raw * scale_num) / scale_den);
}

static void calibrate(void) {
    struct rusage_info_v4 before, after;
    if (usage(getpid(), &before)) exit(2);
    uint64_t start = process_cpu_ns(), end;
    volatile uint64_t value = 1;
    do {
        for (unsigned i = 0; i < 100000; ++i) value = value * 6364136223846793005ULL + 1;
        end = process_cpu_ns();
    } while (end - start < 150000000);
    if (usage(getpid(), &after)) exit(2);
    uint64_t raw = after.ri_user_time + after.ri_system_time - before.ri_user_time - before.ri_system_time;
    double ratio = (double)raw / (double)(end - start);
    mach_timebase_info_data_t timebase;
    if (mach_timebase_info(&timebase) != KERN_SUCCESS) exit(2);
    const char *unit;
    if (ratio > 0.95 && ratio < 1.05) {
        unit = "nanoseconds";
    } else if (ratio * timebase.numer / timebase.denom > 0.95 &&
               ratio * timebase.numer / timebase.denom < 1.05) {
        scale_num = timebase.numer;
        scale_den = timebase.denom;
        unit = "mach_absolute_ticks";
    } else {
        fprintf(stderr, "unsupported CPU accounting scale %.9f\n", ratio);
        exit(2);
    }
    printf("{\"event\":\"calibration\",\"observer_pid\":%d,\"raw_cpu_delta\":%" PRIu64
           ",\"posix_cpu_delta_ns\":%" PRIu64 ",\"raw_unit\":\"%s\",\"scale_num\":%u,\"scale_den\":%u}\n",
           getpid(), raw, end - start, unit, scale_num, scale_den);
    fflush(stdout);
}

static int record(pid_t pid) {
    struct rusage_info_v4 info;
    if (usage(pid, &info)) {
        printf("{\"pid\":%d,\"error\":%d}", pid, errno);
        return 1;
    }
    printf("{\"pid\":%d,\"start_abstime\":%" PRIu64 ",\"physical_footprint_bytes\":%" PRIu64
           ",\"resident_bytes\":%" PRIu64 ",\"wired_bytes\":%" PRIu64
           ",\"lifetime_peak_footprint_bytes\":%" PRIu64 ",\"user_cpu_ns\":%" PRIu64
           ",\"system_cpu_ns\":%" PRIu64 ",\"disk_read_bytes\":%" PRIu64 ",\"disk_write_bytes\":%" PRIu64 "}",
           pid, info.ri_proc_start_abstime, info.ri_phys_footprint, info.ri_resident_size,
           info.ri_wired_size, info.ri_lifetime_max_phys_footprint, nanos(info.ri_user_time),
           nanos(info.ri_system_time), info.ri_diskio_bytesread, info.ri_diskio_byteswritten);
    return 0;
}

static size_t descendants(pid_t root, pid_t *pids, size_t capacity) {
    size_t count = 1;
    pids[0] = root;
    for (size_t cursor = 0; cursor < count; ++cursor) {
        pid_t children[256];
        errno = 0;
        // Unlike proc_listpids, this wrapper returns a PID count, not bytes.
        int nchildren = proc_listchildpids(pids[cursor], children, sizeof(children));
        if (nchildren < 0 || nchildren >= 256 || (nchildren == 0 && errno && errno != ESRCH)) exit(2);
        for (size_t i = 0; i < (size_t)nchildren; ++i) {
            if (children[i] < 1 || children[i] == getpid()) continue;
            int seen = 0;
            for (size_t j = 0; j < count; ++j) seen |= children[i] == pids[j];
            if (seen) continue;
            if (count == capacity) exit(2);
            pids[count++] = children[i];
        }
    }
    return count;
}

int main(void) {
    calibrate();
    char line[4096];
    while (fgets(line, sizeof(line), stdin)) {
        if (!strchr(line, '\n')) return 2;
        pid_t pids[256];
        size_t count = 0;
        char *save, *token = strtok_r(line, " \t\r\n", &save);
        int tree = token && !strcmp(token, "tree");
        if (tree) token = strtok_r(NULL, " \t\r\n", &save);
        while (token) {
            char *end;
            errno = 0;
            long pid = strtol(token, &end, 10);
            if (errno || *end || pid < 1 || pid > INT_MAX || count == 256) return 2;
            pids[count++] = (pid_t)pid;
            token = strtok_r(NULL, " \t\r\n", &save);
        }
        if (tree) {
            if (count != 1) return 2;
            count = descendants(pids[0], pids, 256);
        }
        printf("{\"processes\":[");
        for (size_t i = 0; i < count; ++i) {
            if (i) putchar(',');
            record(pids[i]);
        }
        printf("],\"observer\":");
        record(getpid());
        printf("}\n");
        fflush(stdout);
    }
    return ferror(stdin) ? 2 : 0;
}
