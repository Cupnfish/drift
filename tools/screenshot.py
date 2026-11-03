"""开发辅助：截取全屏/窗口到 .preview/ 下，供核对 GUI。"""
import sys
import pathlib
import subprocess

import ctypes
import ctypes.wintypes as wt

user32 = ctypes.windll.user32
gdi32 = ctypes.windll.gdi32


def find_window_by_pid(pid: int):
    """按进程 PID 找 drift-gui 的可见主窗口。"""
    result = []

    def cb(h, _):
        if not user32.IsWindowVisible(h):
            return True
        length = user32.GetWindowTextLengthW(h)
        if length <= 0:
            return True
        wpid = wt.DWORD()
        user32.GetWindowThreadProcessId(h, ctypes.byref(wpid))
        if wpid.value == pid:
            result.append(h)
        return True

    WNDENUMPROC = ctypes.WINFUNCTYPE(ctypes.c_bool, wt.HWND, wt.LPARAM)
    user32.EnumWindows(WNDENUMPROC(cb), 0)
    return result[0] if result else None


def find_window(prefix: str):
    """找标题以 prefix 开头、且属于 drift-gui.exe 的可见窗口。"""
    matches = []

    def cb(h, _):
        if not user32.IsWindowVisible(h):
            return True
        length = user32.GetWindowTextLengthW(h)
        if length <= 0:
            return True
        buf = ctypes.create_unicode_buffer(length + 1)
        user32.GetWindowTextW(h, buf, length + 1)
        if buf.value.lower().startswith(prefix.lower()):
            pid = wt.DWORD()
            user32.GetWindowThreadProcessId(h, ctypes.byref(pid))
            matches.append((h, pid.value, buf.value))
        return True

    WNDENUMPROC = ctypes.WINFUNCTYPE(ctypes.c_bool, wt.HWND, wt.LPARAM)
    user32.EnumWindows(WNDENUMPROC(cb), 0)
    if not matches:
        return None
    # 优先属于 drift-gui.exe 的窗口
    for h, pid, title in matches:
        try:
            out = subprocess.run(
                ["tasklist", "/FI", f"PID eq {pid}", "/FO", "CSV", "/NH"],
                capture_output=True, text=True, timeout=5,
            ).stdout.strip()
            if "drift-gui" in out.lower():
                print(f"window: {title!r} pid={pid} (drift-gui)")
                return h
        except Exception:
            pass
    print("candidates:", [(t, p) for _, p, t in matches])
    return matches[0][0]


def capture(hwnd, out: pathlib.Path):
    user32.SetProcessDPIAware()
    rect = wt.RECT()
    user32.GetWindowRect(hwnd, ctypes.byref(rect))
    w, h = rect.right - rect.left, rect.bottom - rect.top
    hdc = user32.GetWindowDC(hwnd)
    mem = gdi32.CreateCompatibleDC(hdc)
    bmp = gdi32.CreateCompatibleBitmap(hdc, w, h)
    gdi32.SelectObject(mem, bmp)
    # PW_RENDERFULLCONTENT = 2：直接渲染窗口自身内容（DWM 内容），不怕遮挡/错位
    user32.PrintWindow(hwnd, mem, 2)
    out.parent.mkdir(parents=True, exist_ok=True)
    # 保存为 BMP 再用 PowerShell 转 PNG 太繁琐；直接走 PrintWindow + PIL 不可用时落 BMP
    class BMPFILEHEADER(ctypes.Structure):
        _pack_ = 1
        _fields_ = [("bfType", ctypes.c_uint16), ("bfSize", ctypes.c_uint32),
                    ("bfReserved1", ctypes.c_uint16), ("bfReserved2", ctypes.c_uint16),
                    ("bfOffBits", ctypes.c_uint32)]

    class BMPINFOHEADER(ctypes.Structure):
        _fields_ = [("biSize", ctypes.c_uint32), ("biWidth", ctypes.c_int32),
                    ("biHeight", ctypes.c_int32), ("biPlanes", ctypes.c_uint16),
                    ("biBitCount", ctypes.c_uint16), ("biCompression", ctypes.c_uint32),
                    ("biSizeImage", ctypes.c_uint32), ("biXPelsPerMeter", ctypes.c_int32),
                    ("biYPelsPerMeter", ctypes.c_int32), ("biClrUsed", ctypes.c_uint32),
                    ("biClrImportant", ctypes.c_uint32)]

    bmi = BMPINFOHEADER(ctypes.sizeof(BMPINFOHEADER), w, h, 1, 32, 0, 0, 0, 0, 0, 0)
    pixel_data = ctypes.create_string_buffer(w * h * 4)
    gdi32.GetDIBits(mem, bmp, 0, h, pixel_data, ctypes.byref(bmi), 0)
    header_size = ctypes.sizeof(BMPFILEHEADER) + ctypes.sizeof(BMPINFOHEADER)
    file_header = BMPFILEHEADER(0x4D42, header_size + len(pixel_data.raw), 0, 0, header_size)
    with open(out, "wb") as f:
        f.write(file_header)
        f.write(bmi)
        f.write(pixel_data.raw)
    gdi32.DeleteObject(bmp)
    gdi32.DeleteDC(mem)
    user32.ReleaseDC(hwnd, hdc)
    print(f"saved {out} ({w}x{h})")


if __name__ == "__main__":
    title = sys.argv[1] if len(sys.argv) > 1 else "Drift"
    out = pathlib.Path(sys.argv[2]) if len(sys.argv) > 2 else pathlib.Path(".preview/window.bmp")
    pid = int(sys.argv[3]) if len(sys.argv) > 3 else None
    if pid:
        hwnd = find_window_by_pid(pid)
    else:
        hwnd = find_window(title)
    if not hwnd:
        sys.exit(f"window not found: {title} pid={pid}")
    capture(hwnd, out)
