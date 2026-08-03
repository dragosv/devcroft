/* Authoritative Landlock availability check.
 *
 * landlock_create_ruleset(NULL, 0, LANDLOCK_CREATE_RULESET_VERSION) returns
 * the ABI version supported by the running kernel, or -1. This is the only
 * check that answers the question that matters — whether task 1.1 can run
 * here — because a container can have the syscall present but blocked by
 * seccomp, or a kernel built without Landlock, or Landlock absent from the
 * boot-time lsm= list. All three show up here as a failure.
 */
#include <errno.h>
#include <stdio.h>
#include <string.h>
#include <sys/syscall.h>
#include <unistd.h>

#ifndef __NR_landlock_create_ruleset
#define __NR_landlock_create_ruleset 444
#endif

#ifndef LANDLOCK_CREATE_RULESET_VERSION
#define LANDLOCK_CREATE_RULESET_VERSION (1U << 0)
#endif

int main(void) {
    long abi = syscall(__NR_landlock_create_ruleset, NULL, 0,
                       LANDLOCK_CREATE_RULESET_VERSION);
    if (abi < 0) {
        int e = errno;
        fprintf(stderr, "landlock: UNAVAILABLE (%s)\n", strerror(e));
        switch (e) {
        case ENOSYS:
            fputs("  kernel has no Landlock support\n", stderr);
            break;
        case EOPNOTSUPP:
            fputs("  Landlock is compiled in but disabled at boot "
                  "(missing from the lsm= list)\n", stderr);
            break;
        case EPERM:
            fputs("  syscall blocked — try running the container with "
                  "--security-opt seccomp=unconfined\n", stderr);
            break;
        default:
            break;
        }
        return 1;
    }
    printf("landlock: ABI %ld\n", abi);
    return 0;
}
