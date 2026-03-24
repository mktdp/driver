#ifndef MKTDP_WINDOWS_UNISTD_H
#define MKTDP_WINDOWS_UNISTD_H

#ifdef _WIN32
#include <io.h>
#include <process.h>
#include <sys/types.h>

typedef int pid_t;
#define getpid _getpid
#define access _access
#define close _close
#define read _read
#define write _write
#define lseek _lseek
#define unlink _unlink
#endif

#endif /* MKTDP_WINDOWS_UNISTD_H */
