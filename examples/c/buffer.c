#include <stdio.h>
#include <stdlib.h>

int main(int argc, char **argv) {
    const size_t count = 4;
    if (argc != 2) return 2;
    char *end = NULL;
    unsigned long index = strtoul(argv[1], &end, 10);
    if (!end || *end != '\0') return 2;
    volatile int *values = malloc(count * sizeof(*values));
    if (!values) return 2;
    /* Deliberate bug: index == count is outside the allocation. */
    if (index <= count) values[index] = 42;
    puts("case finished");
    free((void *)values);
    return 0;
}
