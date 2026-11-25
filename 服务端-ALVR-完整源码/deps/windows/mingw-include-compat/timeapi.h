#pragma once
// MinGW 的 mmsystem.h 在某些发行版里不导出 timer API（TIMECAPS/timeBeginPeriod 等）。
// 这里统一补上声明与常量，保证 MSVC 源码在 MinGW 下也能编译。
#include <mmsystem.h>

#ifndef TIMECAPS
typedef struct timecaps_tag {
    UINT wPeriodMin;
    UINT wPeriodMax;
} TIMECAPS, *PTIMECAPS, *LPTIMECAPS;
#endif

#ifndef TIMERR_NOERROR
#define TIMERR_NOERROR 0
#endif
#ifndef TIMERR_NOCANDO
#define TIMERR_NOCANDO 97
#endif

#ifdef __cplusplus
extern "C" {
#endif
WINMMAPI MMRESULT WINAPI timeGetDevCaps(LPTIMECAPS ptc, UINT cbtc);
WINMMAPI UINT WINAPI timeBeginPeriod(UINT uPeriod);
WINMMAPI UINT WINAPI timeEndPeriod(UINT uPeriod);
#ifdef __cplusplus
}
#endif
