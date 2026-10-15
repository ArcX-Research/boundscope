#include <cstdio>
#include <memory>
#include <string_view>

int main(int argc, char **argv) {
    if (argc != 2) return 2;
    const std::string_view mode = argv[1];
    if (mode != "keep-alive" && mode != "release-early") return 2;

    auto owner = std::make_unique<int>(42);
    volatile int *borrowed = owner.get();
    // Deliberate bug: releasing the owner leaves borrowed pointing to freed memory.
    if (mode == "release-early") owner.reset();
    std::printf("%d\n", *borrowed);
    return 0;
}
