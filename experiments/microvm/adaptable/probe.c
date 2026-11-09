#include <stdio.h>
#include <string.h>
#include <zlib.h>

int main(void) {
    const unsigned char input[] = "devcroft pinned native dependency reconstruction";
    unsigned char compressed[256], restored[256];
    uLongf packed = sizeof(compressed), unpacked = sizeof(restored);
    if (compress(compressed, &packed, input, sizeof(input)) != Z_OK ||
        uncompress(restored, &unpacked, compressed, packed) != Z_OK ||
        unpacked != sizeof(input) || memcmp(restored, input, sizeof(input))) return 1;
    printf("zlib=%s roundtrip=ok bytes=%lu\n", zlibVersion(), (unsigned long)unpacked);
    return 0;
}
