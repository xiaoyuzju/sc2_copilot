use image::RgbImage;
use thiserror::Error;

const REFERENCE_WIDTH: u32 = 1920;
const REFERENCE_HEIGHT: u32 = 1080;
const REFERENCE_MINIMAP_LEFT: u32 = 27;
const REFERENCE_MINIMAP_TOP: u32 = 807;
const REFERENCE_MINIMAP_WIDTH: u32 = 264;
const REFERENCE_MINIMAP_HEIGHT: u32 = 259;
pub(crate) const MINIMAP_WIDTH: u32 = REFERENCE_MINIMAP_WIDTH;
pub(crate) const MINIMAP_HEIGHT: u32 = REFERENCE_MINIMAP_HEIGHT;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub(crate) enum CaptureError {
    #[error("未找到 SC2 游戏窗口")]
    WindowNotFound,
    #[error("SC2 不在前台")]
    WindowNotForeground,
    #[error("SC2 窗口已最小化")]
    WindowMinimized,
    #[error("当前 SC2 客户区不是已支持的 16:9 布局")]
    UnsupportedLayout,
    #[error("当前显示器旋转方向尚不支持")]
    UnsupportedRotation,
    #[error("小地图区域缺少可验证的游戏画面内容")]
    InvalidMinimap,
    #[cfg(not(windows))]
    #[error("当前平台不支持实时画面采集")]
    UnsupportedPlatform,
    #[error("画面采集失败：{0}")]
    Backend(String),
}

pub(crate) struct Sc2MinimapCapture {
    #[cfg(windows)]
    window: Option<windows::Win32::Foundation::HWND>,
    #[cfg(windows)]
    backend: Option<windows_capture::DxgiCapture>,
}

impl Sc2MinimapCapture {
    pub(crate) fn new() -> Self {
        Self {
            #[cfg(windows)]
            window: None,
            #[cfg(windows)]
            backend: None,
        }
    }

    #[cfg(not(windows))]
    pub(crate) fn capture(&mut self) -> Result<Option<RgbImage>, CaptureError> {
        Err(CaptureError::UnsupportedPlatform)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MinimapLayout {
    left: u32,
    top: u32,
    width: u32,
    height: u32,
}

impl MinimapLayout {
    fn for_client(width: u32, height: u32) -> Option<Self> {
        if width < 1280 || height < 720 || width * 9 != height * 16 {
            return None;
        }

        let scale_x = |value: u32| scale(value, width, REFERENCE_WIDTH);
        let scale_y = |value: u32| scale(value, height, REFERENCE_HEIGHT);
        let left = scale_x(REFERENCE_MINIMAP_LEFT);
        let top = scale_y(REFERENCE_MINIMAP_TOP);
        let right = scale_x(REFERENCE_MINIMAP_LEFT + REFERENCE_MINIMAP_WIDTH);
        let bottom = scale_y(REFERENCE_MINIMAP_TOP + REFERENCE_MINIMAP_HEIGHT);
        Some(Self {
            left,
            top,
            width: right - left,
            height: bottom - top,
        })
    }
}

fn scale(value: u32, actual: u32, reference: u32) -> u32 {
    ((u64::from(value) * u64::from(actual) + u64::from(reference / 2)) / u64::from(reference))
        as u32
}

fn has_plausible_minimap_content(image: &RgbImage) -> bool {
    if image.dimensions() != (MINIMAP_WIDTH, MINIMAP_HEIGHT) {
        return false;
    }

    let mut histogram = [0_u32; 256];
    for pixel in image.pixels() {
        let [red, green, blue] = pixel.0.map(u32::from);
        let luma = (54 * red + 183 * green + 19 * blue) / 256;
        histogram[luma as usize] += 1;
    }
    let Some(minimum) = histogram.iter().position(|count| *count > 0) else {
        return false;
    };
    let maximum = histogram
        .iter()
        .rposition(|count| *count > 0)
        .unwrap_or(minimum);
    if maximum.saturating_sub(minimum) < 20 {
        return false;
    }

    let contrasting_pixels = histogram[(minimum + 12).min(255)..]
        .iter()
        .copied()
        .sum::<u32>();
    contrasting_pixels >= image.width() * image.height() / 50
}

#[cfg(windows)]
mod windows_capture {
    use std::{path::Path, ptr, slice};

    use image::{Rgb, RgbImage};
    use windows::{
        Win32::{
            Foundation::{CloseHandle, HMODULE, HWND, LPARAM, POINT, RECT},
            Graphics::{
                Direct3D::D3D_DRIVER_TYPE_UNKNOWN,
                Direct3D11::{
                    D3D11_BIND_FLAG, D3D11_BOX, D3D11_CPU_ACCESS_READ,
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE,
                    D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
                    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
                },
                Dxgi::{
                    Common::{
                        DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM,
                        DXGI_FORMAT_R10G10B10A2_UNORM, DXGI_FORMAT_R16G16B16A16_FLOAT,
                        DXGI_MODE_ROTATION_IDENTITY,
                    },
                    CreateDXGIFactory1, DXGI_ERROR_NOT_FOUND, DXGI_ERROR_UNSUPPORTED,
                    DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO, IDXGIAdapter1, IDXGIFactory1,
                    IDXGIOutput5, IDXGIOutputDuplication, IDXGIResource,
                },
                Gdi::{ClientToScreen, HMONITOR, MONITOR_DEFAULTTONEAREST, MonitorFromWindow},
            },
            System::Threading::{
                OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
                QueryFullProcessImageNameW,
            },
            UI::WindowsAndMessaging::{
                EnumWindows, GetClientRect, GetForegroundWindow, GetWindowThreadProcessId,
                IsIconic, IsWindow, IsWindowVisible,
            },
        },
        core::{BOOL, Interface, PWSTR},
    };

    use super::{
        CaptureError, MINIMAP_HEIGHT, MINIMAP_WIDTH, MinimapLayout, Sc2MinimapCapture,
        has_plausible_minimap_content,
    };

    struct TargetWindow {
        monitor: HMONITOR,
        origin: POINT,
        width: u32,
        height: u32,
    }

    pub(super) struct DxgiCapture {
        monitor: HMONITOR,
        output_rect: RECT,
        device: ID3D11Device,
        context: ID3D11DeviceContext,
        duplication: IDXGIOutputDuplication,
        staging: Option<StagingTexture>,
    }

    struct StagingTexture {
        width: u32,
        height: u32,
        format: DXGI_FORMAT,
        texture: ID3D11Texture2D,
    }

    impl Sc2MinimapCapture {
        pub(crate) fn capture(&mut self) -> Result<Option<RgbImage>, CaptureError> {
            let window = self.resolve_window()?;
            let target = inspect_target_window(window)?;
            let layout = MinimapLayout::for_client(target.width, target.height)
                .ok_or(CaptureError::UnsupportedLayout)?;

            if self
                .backend
                .as_ref()
                .is_none_or(|backend| backend.monitor != target.monitor)
            {
                self.backend = Some(DxgiCapture::new(target.monitor)?);
            }

            let result = self
                .backend
                .as_mut()
                .expect("DXGI backend was initialized")
                .capture(&target, layout);
            match result {
                Ok(Some(image)) if !has_plausible_minimap_content(&image) => {
                    Err(CaptureError::InvalidMinimap)
                }
                Err(error) => {
                    self.backend = None;
                    Err(error)
                }
                result => result,
            }
        }

        fn resolve_window(&mut self) -> Result<HWND, CaptureError> {
            if let Some(window) = self.window {
                if unsafe { IsWindow(Some(window)).as_bool() } {
                    return Ok(window);
                }
                self.window = None;
                self.backend = None;
            }

            let window = find_sc2_window().ok_or(CaptureError::WindowNotFound)?;
            self.window = Some(window);
            Ok(window)
        }
    }

    impl DxgiCapture {
        fn new(monitor: HMONITOR) -> Result<Self, CaptureError> {
            let (adapter, output, output_rect) = find_output(monitor)?;
            let mut device = None;
            let mut context = None;
            unsafe {
                D3D11CreateDevice(
                    &adapter,
                    D3D_DRIVER_TYPE_UNKNOWN,
                    HMODULE::default(),
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                    None,
                    D3D11_SDK_VERSION,
                    Some(&mut device),
                    None,
                    Some(&mut context),
                )
                .map_err(|error| backend_error_at("创建 D3D11 设备", error))?;
            }
            let device =
                device.ok_or_else(|| CaptureError::Backend("D3D11 未返回设备".to_owned()))?;
            let context =
                context.ok_or_else(|| CaptureError::Backend("D3D11 未返回上下文".to_owned()))?;
            let duplication =
                match unsafe { output.DuplicateOutput1(&device, 0, &[DXGI_FORMAT_B8G8R8A8_UNORM]) }
                {
                    Ok(duplication) => duplication,
                    Err(error) if error.code() == DXGI_ERROR_UNSUPPORTED => {
                        unsafe { output.DuplicateOutput(&device) }
                            .map_err(|error| backend_error_at("创建兼容桌面复制会话", error))?
                    }
                    Err(error) => return Err(backend_error_at("创建桌面复制会话", error)),
                };

            Ok(Self {
                monitor,
                output_rect,
                device,
                context,
                duplication,
                staging: None,
            })
        }

        fn capture(
            &mut self,
            target: &TargetWindow,
            layout: MinimapLayout,
        ) -> Result<Option<RgbImage>, CaptureError> {
            let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut resource: Option<IDXGIResource> = None;
            if let Err(error) = unsafe {
                self.duplication
                    .AcquireNextFrame(0, &mut frame_info, &mut resource)
            } {
                if error.code() == DXGI_ERROR_WAIT_TIMEOUT {
                    return Ok(None);
                }
                return Err(backend_error_at("获取桌面帧", error));
            }

            let result = (|| {
                let source = resource
                    .ok_or_else(|| CaptureError::Backend("DXGI 未返回桌面纹理".to_owned()))?
                    .cast()
                    .map_err(|error| backend_error_at("转换桌面纹理接口", error))?;
                self.copy_minimap(source, target, layout)
            })();
            let release_result = unsafe { self.duplication.ReleaseFrame() }
                .map_err(|error| backend_error_at("释放桌面帧", error));
            let image = result?;
            release_result?;
            Ok(Some(image))
        }

        fn copy_minimap(
            &mut self,
            source: ID3D11Texture2D,
            target: &TargetWindow,
            layout: MinimapLayout,
        ) -> Result<RgbImage, CaptureError> {
            let mut source_desc = D3D11_TEXTURE2D_DESC::default();
            unsafe { source.GetDesc(&mut source_desc) };
            if bytes_per_pixel(source_desc.Format).is_none() {
                return Err(CaptureError::Backend(format!(
                    "桌面纹理格式不支持：{:?}",
                    source_desc.Format
                )));
            }

            let crop_left = i64::from(target.origin.x) + i64::from(layout.left)
                - i64::from(self.output_rect.left);
            let crop_top = i64::from(target.origin.y) + i64::from(layout.top)
                - i64::from(self.output_rect.top);
            if crop_left < 0
                || crop_top < 0
                || crop_left + i64::from(layout.width) > i64::from(source_desc.Width)
                || crop_top + i64::from(layout.height) > i64::from(source_desc.Height)
            {
                return Err(CaptureError::UnsupportedLayout);
            }

            self.ensure_staging(layout.width, layout.height, source_desc)?;
            let staging = &self
                .staging
                .as_ref()
                .expect("staging texture was initialized")
                .texture;
            let source_box = D3D11_BOX {
                left: crop_left as u32,
                top: crop_top as u32,
                front: 0,
                right: crop_left as u32 + layout.width,
                bottom: crop_top as u32 + layout.height,
                back: 1,
            };
            unsafe {
                self.context.CopySubresourceRegion(
                    staging,
                    0,
                    0,
                    0,
                    0,
                    &source,
                    0,
                    Some(&source_box),
                );
            }

            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            unsafe {
                self.context
                    .Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                    .map_err(|error| backend_error_at("映射小地图读回纹理", error))?;
            }
            let image = copy_crop(
                mapped,
                source_desc.Format,
                0,
                0,
                layout.width,
                layout.height,
            );
            unsafe { self.context.Unmap(staging, 0) };
            image
        }

        fn ensure_staging(
            &mut self,
            width: u32,
            height: u32,
            source_desc: D3D11_TEXTURE2D_DESC,
        ) -> Result<(), CaptureError> {
            if self.staging.as_ref().is_some_and(|staging| {
                staging.width == width
                    && staging.height == height
                    && staging.format == source_desc.Format
            }) {
                return Ok(());
            }

            let staging_desc = D3D11_TEXTURE2D_DESC {
                Width: width,
                Height: height,
                MipLevels: 1,
                ArraySize: 1,
                Format: source_desc.Format,
                SampleDesc: source_desc.SampleDesc,
                Usage: D3D11_USAGE_STAGING,
                BindFlags: D3D11_BIND_FLAG(0).0 as u32,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                MiscFlags: 0,
            };
            let mut texture = None;
            unsafe {
                self.device
                    .CreateTexture2D(&staging_desc, None, Some(&mut texture))
                    .map_err(|error| backend_error_at("创建小地图读回纹理", error))?;
            }
            self.staging = Some(StagingTexture {
                width,
                height,
                format: source_desc.Format,
                texture: texture
                    .ok_or_else(|| CaptureError::Backend("D3D11 未返回读回纹理".to_owned()))?,
            });
            Ok(())
        }
    }

    fn copy_crop(
        mapped: D3D11_MAPPED_SUBRESOURCE,
        format: DXGI_FORMAT,
        crop_left: u32,
        crop_top: u32,
        crop_width: u32,
        crop_height: u32,
    ) -> Result<RgbImage, CaptureError> {
        let pixel_size = bytes_per_pixel(format)
            .ok_or_else(|| CaptureError::Backend(format!("桌面纹理格式不支持：{format:?}")))?;
        if mapped.pData.is_null() || mapped.RowPitch < pixel_size as u32 {
            return Err(CaptureError::Backend("D3D11 映射结果无效".to_owned()));
        }

        let data = unsafe {
            slice::from_raw_parts(
                mapped.pData.cast::<u8>(),
                mapped.RowPitch as usize * (crop_top + crop_height) as usize,
            )
        };
        Ok(RgbImage::from_fn(MINIMAP_WIDTH, MINIMAP_HEIGHT, |x, y| {
            let source_x = crop_left + ((2 * x + 1) * crop_width) / (2 * MINIMAP_WIDTH);
            let source_y = crop_top + ((2 * y + 1) * crop_height) / (2 * MINIMAP_HEIGHT);
            let offset =
                source_y as usize * mapped.RowPitch as usize + source_x as usize * pixel_size;
            Rgb(decode_pixel(format, &data[offset..offset + pixel_size])
                .expect("format was validated"))
        }))
    }

    fn bytes_per_pixel(format: DXGI_FORMAT) -> Option<usize> {
        match format {
            DXGI_FORMAT_B8G8R8A8_UNORM
            | DXGI_FORMAT_R8G8B8A8_UNORM
            | DXGI_FORMAT_R10G10B10A2_UNORM => Some(4),
            DXGI_FORMAT_R16G16B16A16_FLOAT => Some(8),
            _ => None,
        }
    }

    pub(super) fn decode_pixel(format: DXGI_FORMAT, pixel: &[u8]) -> Option<[u8; 3]> {
        match format {
            DXGI_FORMAT_B8G8R8A8_UNORM if pixel.len() >= 4 => Some([pixel[2], pixel[1], pixel[0]]),
            DXGI_FORMAT_R8G8B8A8_UNORM if pixel.len() >= 4 => Some([pixel[0], pixel[1], pixel[2]]),
            DXGI_FORMAT_R10G10B10A2_UNORM if pixel.len() >= 4 => {
                let packed = u32::from_le_bytes(pixel[..4].try_into().ok()?);
                Some([
                    unorm10_to_u8(packed & 0x3ff),
                    unorm10_to_u8((packed >> 10) & 0x3ff),
                    unorm10_to_u8((packed >> 20) & 0x3ff),
                ])
            }
            DXGI_FORMAT_R16G16B16A16_FLOAT if pixel.len() >= 8 => Some([
                half_to_srgb_u8(&pixel[0..2]),
                half_to_srgb_u8(&pixel[2..4]),
                half_to_srgb_u8(&pixel[4..6]),
            ]),
            _ => None,
        }
    }

    fn unorm10_to_u8(value: u32) -> u8 {
        ((value * 255 + 511) / 1023) as u8
    }

    fn half_to_srgb_u8(bytes: &[u8]) -> u8 {
        let linear = half::f16::from_bits(u16::from_le_bytes([bytes[0], bytes[1]]))
            .to_f32()
            .clamp(0.0, 1.0);
        let srgb = if linear <= 0.003_130_8 {
            linear * 12.92
        } else {
            1.055 * linear.powf(1.0 / 2.4) - 0.055
        };
        (srgb * 255.0).round() as u8
    }

    fn inspect_target_window(window: HWND) -> Result<TargetWindow, CaptureError> {
        if unsafe { GetForegroundWindow() } != window {
            return Err(CaptureError::WindowNotForeground);
        }
        if unsafe { IsIconic(window).as_bool() } {
            return Err(CaptureError::WindowMinimized);
        }

        let mut client_rect = RECT::default();
        unsafe { GetClientRect(window, &mut client_rect) }.map_err(backend_error)?;
        let width = u32::try_from(client_rect.right - client_rect.left)
            .map_err(|_| CaptureError::UnsupportedLayout)?;
        let height = u32::try_from(client_rect.bottom - client_rect.top)
            .map_err(|_| CaptureError::UnsupportedLayout)?;
        let mut origin = POINT::default();
        if !unsafe { ClientToScreen(window, &mut origin).as_bool() } {
            return Err(CaptureError::Backend(
                "无法取得 SC2 客户区屏幕坐标".to_owned(),
            ));
        }
        let monitor = unsafe { MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST) };
        if monitor.0.is_null() {
            return Err(CaptureError::Backend("无法定位 SC2 所在显示器".to_owned()));
        }

        Ok(TargetWindow {
            monitor,
            origin,
            width,
            height,
        })
    }

    fn find_output(monitor: HMONITOR) -> Result<(IDXGIAdapter1, IDXGIOutput5, RECT), CaptureError> {
        let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }.map_err(backend_error)?;
        for adapter_index in 0.. {
            let adapter = match unsafe { factory.EnumAdapters1(adapter_index) } {
                Ok(adapter) => adapter,
                Err(error) if error.code() == DXGI_ERROR_NOT_FOUND => break,
                Err(error) => return Err(backend_error(error)),
            };
            for output_index in 0.. {
                let output = match unsafe { adapter.EnumOutputs(output_index) } {
                    Ok(output) => output,
                    Err(error) if error.code() == DXGI_ERROR_NOT_FOUND => break,
                    Err(error) => return Err(backend_error(error)),
                };
                let desc = unsafe { output.GetDesc() }.map_err(backend_error)?;
                if desc.Monitor != monitor {
                    continue;
                }
                if desc.Rotation != DXGI_MODE_ROTATION_IDENTITY {
                    return Err(CaptureError::UnsupportedRotation);
                }
                return Ok((
                    adapter,
                    output.cast().map_err(backend_error)?,
                    desc.DesktopCoordinates,
                ));
            }
        }
        Err(CaptureError::Backend(
            "找不到 SC2 所在显示器的 DXGI 输出".to_owned(),
        ))
    }

    fn find_sc2_window() -> Option<HWND> {
        let mut window = None;
        let parameter = LPARAM(ptr::from_mut(&mut window) as isize);
        let _ = unsafe { EnumWindows(Some(find_sc2_window_callback), parameter) };
        window
    }

    unsafe extern "system" fn find_sc2_window_callback(window: HWND, parameter: LPARAM) -> BOOL {
        if !unsafe { IsWindowVisible(window).as_bool() } {
            return BOOL(1);
        }

        let mut process_id = 0;
        unsafe { GetWindowThreadProcessId(window, Some(&mut process_id)) };
        if process_name_matches(process_id) {
            let found = unsafe { &mut *(parameter.0 as *mut Option<HWND>) };
            *found = Some(window);
            return BOOL(0);
        }
        BOOL(1)
    }

    fn process_name_matches(process_id: u32) -> bool {
        let Ok(process) =
            (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) })
        else {
            return false;
        };
        let matches = query_process_path(process).is_some_and(|path| {
            Path::new(&path)
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case("SC2_x64.exe"))
        });
        let _ = unsafe { CloseHandle(process) };
        matches
    }

    fn query_process_path(process: windows::Win32::Foundation::HANDLE) -> Option<String> {
        let mut buffer = vec![0_u16; 32_768];
        let mut length = buffer.len() as u32;
        unsafe {
            QueryFullProcessImageNameW(
                process,
                PROCESS_NAME_WIN32,
                PWSTR(buffer.as_mut_ptr()),
                &mut length,
            )
            .ok()?;
        }
        Some(String::from_utf16_lossy(&buffer[..length as usize]))
    }

    fn backend_error(error: windows::core::Error) -> CaptureError {
        CaptureError::Backend(error.to_string())
    }

    fn backend_error_at(operation: &str, error: windows::core::Error) -> CaptureError {
        CaptureError::Backend(format!("{operation}：{error}"))
    }
}

#[cfg(test)]
mod tests {
    use image::{Rgb, RgbImage};

    use super::MinimapLayout;

    #[test]
    fn maps_the_reference_minimap_at_1080p() {
        let layout = MinimapLayout::for_client(1920, 1080).expect("supported layout");

        assert_eq!((layout.left, layout.top), (27, 807));
        assert_eq!((layout.width, layout.height), (264, 259));
    }

    #[test]
    fn scales_the_minimap_at_1440p_and_4k() {
        let layout_1440 = MinimapLayout::for_client(2560, 1440).expect("supported layout");
        assert_eq!((layout_1440.left, layout_1440.top), (36, 1076));
        assert_eq!((layout_1440.width, layout_1440.height), (352, 345));

        let layout_4k = MinimapLayout::for_client(3840, 2160).expect("supported layout");
        assert_eq!((layout_4k.left, layout_4k.top), (54, 1614));
        assert_eq!((layout_4k.width, layout_4k.height), (528, 518));
    }

    #[test]
    fn rejects_unverified_aspect_ratios_and_small_clients() {
        assert_eq!(MinimapLayout::for_client(1920, 1200), None);
        assert_eq!(MinimapLayout::for_client(960, 540), None);
    }

    #[test]
    fn rejects_uniform_or_nearly_empty_minimap_crops() {
        let black =
            RgbImage::from_pixel(super::MINIMAP_WIDTH, super::MINIMAP_HEIGHT, Rgb([0, 0, 0]));
        let mut cursor_only = black.clone();
        for y in 100..105 {
            for x in 100..105 {
                cursor_only.put_pixel(x, y, Rgb([255, 255, 255]));
            }
        }

        assert!(!super::has_plausible_minimap_content(&black));
        assert!(!super::has_plausible_minimap_content(&cursor_only));
    }

    #[test]
    fn accepts_a_varied_minimap_crop() {
        let minimap = RgbImage::from_fn(super::MINIMAP_WIDTH, super::MINIMAP_HEIGHT, |x, y| {
            if (x / 24 + y / 24) % 2 == 0 {
                Rgb([18, 35, 48])
            } else {
                Rgb([72, 92, 54])
            }
        });

        assert!(super::has_plausible_minimap_content(&minimap));
    }

    #[cfg(windows)]
    #[test]
    fn converts_hdr_half_float_sc_rgb_pixels_to_rgb8() {
        use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_R16G16B16A16_FLOAT;

        let red_with_alpha = [0x00, 0x3c, 0x00, 0x00, 0x00, 0x00, 0x00, 0x3c];
        assert_eq!(
            super::windows_capture::decode_pixel(DXGI_FORMAT_R16G16B16A16_FLOAT, &red_with_alpha,),
            Some([255, 0, 0]),
        );
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires an active SC2 match in the foreground"]
    fn captures_a_live_sc2_minimap() {
        let mut capture = super::Sc2MinimapCapture::new();
        for _ in 0..20 {
            match capture.capture() {
                Ok(Some(image)) => {
                    assert_eq!(
                        image.dimensions(),
                        (super::MINIMAP_WIDTH, super::MINIMAP_HEIGHT)
                    );
                    return;
                }
                Ok(None) | Err(super::CaptureError::InvalidMinimap) => {}
                Err(error) => panic!("live SC2 capture failed: {error}"),
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("DXGI returned no valid SC2 minimap frame");
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires a target-map detection window in an active foreground SC2 match"]
    fn observes_a_live_sc2_minimap_ping() {
        use sc2_copilot_vision::{
            MinimapPingRecognizer, PingFrame, PingObservation, UnavailableReason,
        };

        let mut capture = super::Sc2MinimapCapture::new();
        let mut recognizer = MinimapPingRecognizer::default();
        for frame_id in 1..=3_000 {
            let observation = match capture.capture() {
                Ok(Some(image)) => {
                    recognizer.observe(PingFrame::available("live-session", frame_id, &image))
                }
                Ok(None) => {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    continue;
                }
                Err(super::CaptureError::InvalidMinimap) => {
                    recognizer.observe(PingFrame::unavailable(
                        "live-session",
                        frame_id,
                        UnavailableReason::UnsupportedLayout,
                    ))
                }
                Err(error) => panic!("live SC2 capture failed: {error}"),
            };
            if !matches!(observation, PingObservation::NoEvidence) {
                eprintln!("frame {frame_id}: {observation:?}");
            }
            if matches!(observation, PingObservation::Confirmed { .. }) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        panic!("no confirmed minimap ping was observed within five minutes");
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "waits for a fresh foreground match on a supported target map"]
    fn resolves_a_live_target_map_variant_end_to_end() {
        use std::time::{Duration, Instant};

        use sc2_copilot_vision::{
            MinimapPingRecognizer, PingFrame, PingObservation, UnavailableReason, VisionUpdate,
        };

        use crate::{
            LocalSc2HttpClient, MapVariantVision, Sc2Observation, Sc2StateSource, VisionContext,
            vision::VisionPhase,
        };

        let client = LocalSc2HttpClient::new(Duration::from_millis(750))
            .expect("build direct SC2 HTTP client");
        let mut source = Sc2StateSource::new(client);
        let mut capture = super::Sc2MinimapCapture::new();
        let mut recognizer = MinimapPingRecognizer::default();
        let mut vision = MapVariantVision::default();
        let mut frame_id = 0_u64;
        let mut stale_session = None;
        let mut active_session = None;
        let mut valid_frames = 0_u64;
        let deadline = Instant::now() + Duration::from_secs(7 * 60);

        while Instant::now() < deadline {
            let poll = source.poll();
            let Sc2Observation::InGame {
                session_id,
                map_id: Some(map_id),
                game_time_milliseconds,
                ..
            } = poll.observation
            else {
                let _ = vision.update_context(None);
                std::thread::sleep(Duration::from_millis(100));
                continue;
            };
            let context = VisionContext::new(&session_id, &map_id, game_time_milliseconds);
            if let Some(update) = vision.update_context(Some(context)) {
                assert_eq!(
                    update,
                    VisionUpdate::map_variant(
                        &session_id,
                        &map_id,
                        expected_live_variant(&map_id, false),
                    )
                );
                eprintln!(
                    "resolved live map variant after {valid_frames} valid frames: {update:?}"
                );
                return;
            }
            let phase = vision.phase();
            if phase == VisionPhase::Idle {
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
            if phase == VisionPhase::Missed {
                if active_session.as_deref() == Some(session_id.as_str()) {
                    panic!(
                        "target-map window ended without a decision after {valid_frames} valid frames"
                    );
                }
                if stale_session.is_none() {
                    eprintln!(
                        "ignoring stale {map_id} session {session_id} at {game_time_milliseconds} ms"
                    );
                    stale_session = Some(session_id);
                } else if stale_session.as_deref() != Some(&session_id) {
                    panic!(
                        "fresh target-map session was first observed after its detection window"
                    );
                }
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
            if stale_session.as_deref() == Some(&session_id) {
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
            if active_session.as_deref() != Some(session_id.as_str()) {
                eprintln!("observing fresh {map_id} session {session_id}");
                active_session = Some(session_id.clone());
                valid_frames = 0;
            }
            if phase == VisionPhase::Capturing {
                frame_id = frame_id.wrapping_add(1);
                let observation = match capture.capture() {
                    Ok(Some(image)) => {
                        recognizer.observe(PingFrame::available(&session_id, frame_id, &image))
                    }
                    Ok(None) => {
                        std::thread::sleep(Duration::from_millis(100));
                        continue;
                    }
                    Err(super::CaptureError::InvalidMinimap) => {
                        recognizer.observe(PingFrame::unavailable(
                            &session_id,
                            frame_id,
                            UnavailableReason::UnsupportedLayout,
                        ))
                    }
                    Err(error) => panic!("live SC2 capture failed: {error}"),
                };
                if !matches!(observation, PingObservation::Unavailable { .. }) {
                    valid_frames += 1;
                }
                if !matches!(observation, PingObservation::NoEvidence) {
                    eprintln!(
                        "{map_id} at {game_time_milliseconds} ms, frame {frame_id}: {observation:?}"
                    );
                }
                if let Some(update) = vision.observe_ping(observation) {
                    assert_eq!(
                        update,
                        VisionUpdate::map_variant(
                            &session_id,
                            &map_id,
                            expected_live_variant(&map_id, true),
                        )
                    );
                    eprintln!(
                        "resolved live map variant from a confirmed ping after {valid_frames} valid frames: {update:?}"
                    );
                    return;
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        panic!("timed out waiting for a fresh target-map detection window");
    }

    fn expected_live_variant(map_id: &str, ping_present: bool) -> &'static str {
        match (map_id, ping_present) {
            ("void-rifts", true) | ("temple-of-the-past", false) => "layout-a",
            ("void-rifts", false) | ("temple-of-the-past", true) => "layout-b",
            _ => panic!("unsupported live calibration map: {map_id}"),
        }
    }
}
