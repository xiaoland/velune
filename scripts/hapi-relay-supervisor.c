#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <signal.h>
#include <spawn.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

extern char **environ;

static volatile sig_atomic_t stopping;

static void request_stop(int signal_number) {
    (void)signal_number;
    stopping = 1;
}

static int child_result(int status) {
    if (WIFEXITED(status)) {
        return WEXITSTATUS(status);
    }
    return WIFSIGNALED(status) ? 128 + WTERMSIG(status) : 1;
}

int main(int argc, char **argv) {
    if (argc < 2) {
        return 2;
    }
    struct sigaction action = {.sa_handler = request_stop};
    sigemptyset(&action.sa_mask);
    if (sigaction(SIGTERM, &action, NULL) != 0 ||
        sigaction(SIGINT, &action, NULL) != 0) {
        return 2;
    }
    posix_spawn_file_actions_t files;
    if (posix_spawn_file_actions_init(&files) != 0) {
        return 2;
    }
    int error = posix_spawn_file_actions_addopen(&files, STDIN_FILENO, "/dev/null", O_RDONLY, 0);
    pid_t child = 0;
    if (error == 0 && !stopping) {
        error = posix_spawn(&child, argv[1], &files, NULL, argv + 1, environ);
    }
    posix_spawn_file_actions_destroy(&files);
    if (error != 0 || child <= 0) {
        return 2;
    }

    // The App owns the sole write end. Even SIGKILL closes that end.
    struct pollfd owner = {.fd = STDIN_FILENO, .events = POLLIN};
    int status;
    while (!stopping) {
        pid_t exited = waitpid(child, &status, WNOHANG);
        if (exited == child) {
            return child_result(status);
        }
        if (exited == -1 && errno != EINTR) {
            return 1;
        }
        int ready = poll(&owner, 1, 100);
        if (ready > 0 || (ready < 0 && errno != EINTR)) {
            break;
        }
    }

    // Until we reap the child, the PID cannot be reused for another process.
    kill(child, SIGTERM);
    for (int attempt = 0; attempt < 20; attempt++) {
        pid_t exited = waitpid(child, &status, WNOHANG);
        if (exited == child || (exited == -1 && errno != EINTR)) {
            return 0;
        }
        nanosleep(&(struct timespec){.tv_nsec = 100000000}, NULL);
    }
    kill(child, SIGKILL);
    while (waitpid(child, &status, 0) < 0 && errno == EINTR) {}
    return 0;
}
