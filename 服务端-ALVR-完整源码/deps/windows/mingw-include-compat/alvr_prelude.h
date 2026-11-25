#pragma once
// MSVC's standard library/windows headers pull these in transitively.
// MinGW does not, so force-include them for the Windows cross build.
#include <mutex>
#include <condition_variable>
#include <thread>
#include <atomic>
#include <share.h>
#include <stdio.h>
#ifndef vscwprintf
#define vscwprintf _vscwprintf
#endif
#ifndef vscprintf
#define vscprintf _vscprintf
#endif
