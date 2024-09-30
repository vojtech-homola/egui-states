# ruff: noqa: D107 D101
from egui_pysync import enums, structures, types
from egui_pysync.core import StateServer


class ScanImage:
    def __init__(self, server: StateServer, signals_manager: structures._SignalsManager):
        self.image = structures.ValueImage(11, server, signals_manager)
        self.scale = structures.Value[float](12, server, signals_manager)
        self.position = structures.Value[tuple[float, float]](13, server, signals_manager)
        self.hist_min = structures.Value[float](14, server, signals_manager)
        self.hist_max = structures.Value[float](15, server, signals_manager)


class Optics:
    def __init__(self, server: StateServer, signals_manager: structures._SignalsManager):
        self.optical_mode = structures.ValueEnum(16, server, signals_manager, enums.OpticalMode)
        self.microscope_mode = structures.ValueEnum(17, server, signals_manager, enums.MicroscopeMode)
        self.mic_state = structures.SignalEmpty(18, server, signals_manager)
        self.mic_state_busy = structures.Value[bool](19, server, signals_manager)
        self.current = structures.Value[float](20, server, signals_manager)
        self.angle = structures.Value[float](21, server, signals_manager)
        self.d50_spot_size = structures.Value[float](22, server, signals_manager)
        self.keep_adjutments = structures.Value[bool](23, server, signals_manager)


class Stem:
    def __init__(self, server: StateServer, signals_manager: structures._SignalsManager):
        self.channel = structures.ValueEnum(24, server, signals_manager, enums.StemChannel)
        self.tools = structures.ValueEnum(25, server, signals_manager, enums.StemTools)
        self.histogram = structures.Value[bool](26, server, signals_manager)
        self.cross = structures.Value[bool](27, server, signals_manager)
        self.xy_range = structures.Value[bool](28, server, signals_manager)
        self.xy_orientation = structures.Value[bool](29, server, signals_manager)
        self.size_bar = structures.Value[bool](30, server, signals_manager)
        self.info = structures.Value[bool](31, server, signals_manager)


class Scanning:
    def __init__(self, server: StateServer, signals_manager: structures._SignalsManager):
        self.fov = structures.Value[float](32, server, signals_manager)
        self.pixel_time = structures.Value[float](33, server, signals_manager)
        self.pixel_count = structures.ValueEnum(34, server, signals_manager, enums.PixelCount)
        self.rotation = structures.Value[float](35, server, signals_manager)
        self.acq_state = structures.ValueEnum(36, server, signals_manager, enums.AcquireState)
        self.blanker = structures.ValueEnum(37, server, signals_manager, enums.Blanker)
        self.off_axis = structures.Value[bool](38, server, signals_manager)


class Camera:
    def __init__(self, server: StateServer, signals_manager: structures._SignalsManager):
        self.roi_mode = structures.ValueEnum(39, server, signals_manager, enums.ROIMode)
        self.exposure = structures.Value[float](40, server, signals_manager)
        self.angle = structures.Value[float](41, server, signals_manager)
        self.acq_state = structures.ValueEnum(42, server, signals_manager, enums.AcquireState)
        self.df_range = structures.Value[tuple[float, float]](43, server, signals_manager)
        self.bf_range = structures.Value[tuple[float, float]](44, server, signals_manager)


class Detectors:
    def __init__(self, server: StateServer, signals_manager: structures._SignalsManager):
        self.bf = structures.Value[bool](45, server, signals_manager)
        self.bf_busy = structures.Value[bool](46, server, signals_manager)
        self.bf_gain = structures.Value[float](47, server, signals_manager)
        self.df = structures.Value[bool](48, server, signals_manager)
        self.df_busy = structures.Value[bool](49, server, signals_manager)
        self.df_gain = structures.Value[float](50, server, signals_manager)


class Stem4D:
    def __init__(self, server: StateServer, signals_manager: structures._SignalsManager):
        self.pixel_time = structures.Value[float](51, server, signals_manager)
        self.pixel_count = structures.ValueEnum(52, server, signals_manager, enums.PixelCount)
        self.button = structures.SignalEmpty(53, server, signals_manager)
        self.button_text = structures.Value[str](54, server, signals_manager)
        self.state_text = structures.Value[str](55, server, signals_manager)
        self.progress = structures.Value[float](56, server, signals_manager)
        self.auto_retract = structures.Value[bool](57, server, signals_manager)


class StemAdjustments:
    def __init__(self, server: StateServer, signals_manager: structures._SignalsManager):
        self.adjust_type = structures.ValueEnum(58, server, signals_manager, enums.StemAdjust)
        self.ele_adjust = structures.ValueEnum(59, server, signals_manager, enums.EleAdjust)
        self.scan = structures.Value[tuple[float, float]](60, server, signals_manager)
        self.precession = structures.Value[tuple[float, float]](61, server, signals_manager)
        self.tilt_wobbler = structures.Value[bool](62, server, signals_manager)
        self.wobbler_direction = structures.ValueEnum(63, server, signals_manager, enums.AxisDirection)
        self.wobbler_angle = structures.Value[float](64, server, signals_manager)
        self.focus = structures.Value[float](65, server, signals_manager)
        self.autofocus = structures.SignalEmpty(66, server, signals_manager)
        self.focus_type = structures.ValueEnum(67, server, signals_manager, enums.FocusType)
        self.stigmator = structures.Value[tuple[float, float]](68, server, signals_manager)
        self.stig_auto = structures.SignalEmpty(69, server, signals_manager)


class Stage:
    def __init__(self, server: StateServer, signals_manager: structures._SignalsManager):
        self.stop = structures.SignalEmpty(70, server, signals_manager)
        self.x = structures.Value[float](71, server, signals_manager)
        self.y = structures.Value[float](72, server, signals_manager)
        self.z = structures.Value[float](73, server, signals_manager)
        self.alpha = structures.Value[float](74, server, signals_manager)
        self.beta = structures.Value[float](75, server, signals_manager)
        self.set_zero = structures.SignalEmpty(76, server, signals_manager)
        self.xyz_backlash = structures.Value[bool](77, server, signals_manager)
        self.backlash = structures.Value[bool](78, server, signals_manager)


class Diffraction:
    def __init__(self, server: StateServer, signals_manager: structures._SignalsManager):
        self.channel = structures.ValueEnum(79, server, signals_manager, enums.DiffChannel)
        self.tools = structures.ValueEnum(80, server, signals_manager, enums.DiffTools)
        self.histogram = structures.Value[bool](81, server, signals_manager)
        self.cross = structures.Value[bool](82, server, signals_manager)
        self.circle = structures.Value[bool](83, server, signals_manager)
        self.xy_range = structures.Value[bool](84, server, signals_manager)
        self.xy_orientation = structures.Value[bool](85, server, signals_manager)
        self.size_bar = structures.Value[bool](86, server, signals_manager)
        self.info = structures.Value[bool](87, server, signals_manager)
        self.mask_type = structures.ValueEnum(88, server, signals_manager, enums.MaskType)
        self.mask_segments = structures.Value[int](89, server, signals_manager)


class DiffAdjustments:
    def __init__(self, server: StateServer, signals_manager: structures._SignalsManager):
        self.adjust_type = structures.ValueEnum(90, server, signals_manager, enums.DiffAdjust)
        self.tilt = structures.ValueEnum(91, server, signals_manager, enums.EleTilt)
        self.illumination = structures.Value[tuple[float, float]](92, server, signals_manager)
        self.projection = structures.Value[tuple[float, float]](93, server, signals_manager)


class Precession:
    def __init__(self, server: StateServer, signals_manager: structures._SignalsManager):
        self.enabled = structures.Value[bool](94, server, signals_manager)
        self.image = structures.Value[bool](95, server, signals_manager)
        self.angle = structures.Value[float](96, server, signals_manager)
        self.freq = structures.Value[float](97, server, signals_manager)
        self.cycles = structures.Value[int](98, server, signals_manager)
        self.pivot_tune = structures.Value[bool](99, server, signals_manager)
        self.fov_factor = structures.Value[float](100, server, signals_manager)
        self.precess_metric = structures.ValueEnum(101, server, signals_manager, enums.PrecessMetric)
        self.h_xx = structures.Value[float](102, server, signals_manager)
        self.h_xy = structures.Value[float](103, server, signals_manager)
        self.h_yx = structures.Value[float](104, server, signals_manager)
        self.h_yy = structures.Value[float](105, server, signals_manager)
        self.deprecess_tune = structures.Value[bool](106, server, signals_manager)
        self.deprecess_factor = structures.Value[float](107, server, signals_manager)
        self.deprecess_metric = structures.ValueEnum(108, server, signals_manager, enums.DeprecessMetric)
        self.deprecess_enabled = structures.Value[bool](109, server, signals_manager)
        self.a_xx = structures.Value[float](110, server, signals_manager)
        self.a_xy = structures.Value[float](111, server, signals_manager)
        self.a_yx = structures.Value[float](112, server, signals_manager)
        self.a_yy = structures.Value[float](113, server, signals_manager)


class Xray:
    def __init__(self, server: StateServer, signals_manager: structures._SignalsManager):
        self.edx0 = structures.Value[bool](114, server, signals_manager)
        self.edx1 = structures.Value[bool](115, server, signals_manager)
        self.calibrate = structures.SignalEmpty(116, server, signals_manager)
        self.status = structures.ValueStatic[str](117, server)
        self.add_element = structures.Signal[str](118, server, signals_manager)
        self.elements = structures.ValueDict[int, bool](119, server, signals_manager)
        self.setter = structures.Signal[types.Element](120, server, signals_manager)


class States:
    def __init__(self, server: StateServer, signals_manager: structures._SignalsManager):
        self._server = server
        self._signals_manager = signals_manager

        self.scan_image = ScanImage(server, signals_manager)
        self.optics = Optics(server, signals_manager)
        self.stem = Stem(server, signals_manager)
        self.scanning = Scanning(server, signals_manager)
        self.camera = Camera(server, signals_manager)
        self.detectors = Detectors(server, signals_manager)
        self.stem4d = Stem4D(server, signals_manager)
        self.stem_adjustments = StemAdjustments(server, signals_manager)
        self.stage = Stage(server, signals_manager)
        self.diffraction = Diffraction(server, signals_manager)
        self.diff_adjustments = DiffAdjustments(server, signals_manager)
        self.precession = Precession(server, signals_manager)
        self.xray = Xray(server, signals_manager)

        self.view = structures.ValueEnum(10, server, signals_manager, enums.View)

        signals_manager.close_registration()

    def update(self, duration: float | None = None) -> None:
        """Update the UI.

        Args:
            duration: The duration of the update.
        """
        self._server.update(duration)
