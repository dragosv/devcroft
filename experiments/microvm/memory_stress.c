// Fixed diagnostic workload, compiled and executed only in the guest.
#include <stdio.h>
#include <stdint.h>
#include <sys/mman.h>
#include <time.h>
#include <unistd.h>

int main(void) {
    const size_t bytes = 256UL * 1024 * 1024;
    volatile uint64_t *memory = mmap(NULL, bytes, PROT_READ | PROT_WRITE,
                                         MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if ((void *)memory == MAP_FAILED) return 1;
    // Fill every word, not just one byte per page: mostly-zero pages would
    // make host compression obscure actual allocate/touch/free behavior.
    uint64_t value = 0x9e3779b97f4a7c15ULL;
    for (size_t i = 0; i < bytes / sizeof(*memory); ++i) {
        value ^= value >> 12;
        value ^= value << 25;
        value ^= value >> 27;
        memory[i] = value * 2685821657736338717ULL;
    }
    puts("TOUCHED 268435456");
    fflush(stdout);
    struct timespec pause = {.tv_sec = 2};
    while (nanosleep(&pause, &pause)) {}
    if (munmap((void *)memory, bytes)) return 2;
    puts("FREED 268435456");
    return 0;
}
