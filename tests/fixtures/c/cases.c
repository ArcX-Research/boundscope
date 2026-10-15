#define _POSIX_C_SOURCE 200809L
#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

static void wait_forever(void) {
    const struct timespec delay = {0, 10000000};
    for (;;) nanosleep(&delay, NULL);
}

int main(int argc, char **argv) {
    if (argc != 2) return 7;
    if (strcmp(argv[1], "safe") == 0) {
        char input[32];
        if (!fgets(input, sizeof(input), stdin)) return 8;
        printf("received: %s", input);
        return 0;
    }
    if (strcmp(argv[1], "oob") == 0) {
        volatile int *p = malloc(sizeof(*p));
        if (!p) return 9;
        p[argc] = 42;
        free((void *)p);
        return 0;
    }
    if (strcmp(argv[1], "uaf") == 0) {
        volatile int *p = malloc(sizeof(*p));
        if (!p) return 9;
        *p = 3;
        free((void *)p);
        printf("%d\n", *p);
        return 0;
    }
    if (strcmp(argv[1], "overflow") == 0) {
        volatile int value = INT_MAX;
        printf("%d\n", value + 1);
        return 0;
    }
    if (strcmp(argv[1], "timeout") == 0) wait_forever();
    if (strcmp(argv[1], "flood") == 0) {
        char output[8192];
        memset(output, 'x', sizeof(output));
        for (;;) { if (fwrite(output, 1, sizeof(output), stdout) != sizeof(output)) return 9; }
    }
    if (strcmp(argv[1], "child") == 0 || strcmp(argv[1], "background") == 0 || strcmp(argv[1], "finishing") == 0) {
        pid_t child = fork();
        if (child < 0) return 9;
        if (child == 0) {
            if (strcmp(argv[1], "finishing") == 0) {
                const struct timespec delay = {0, 50000000};
                nanosleep(&delay, NULL);
                return 0;
            }
            wait_forever();
        }
        FILE *marker = fopen("descendant.pid", "w");
        if (!marker) return 9;
        if (fprintf(marker, "%ld\n", (long)child) < 0 || fclose(marker) != 0) return 9;
        printf("%ld\n", (long)child);
        fflush(stdout);
        if (strcmp(argv[1], "child") != 0) return 0;
        wait_forever();
    }
    return 7;
}
