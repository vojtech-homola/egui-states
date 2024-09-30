# ruff: noqa: D101
from enum import Enum


class View(Enum):
    Survey = 0
    Navigation = 1


class OpticalMode(Enum):
    Off = 0
    STEM = 1
    LM = 2
    Parallel = 3


class MicroscopeMode(Enum):
    Standby = 0
    Ready = 1
    Acquiring = 2


class StemChannel(Enum):
    BF = 0
    DF = 1
    EDX = 2


class DiffChannel(Enum):
    CAMERA = 0
    FFT = 1


class StemTools(Enum):
    Not = 0
    Pointer = 1
    Rectangle = 2
    Distance = 3
    Angle = 4


class DiffTools(Enum):
    Not = 0
    Mask = 1
    Distance = 2
    Angle = 3
    Ronchigram = 4


class MaskType(Enum):
    Full = 0
    Angular = 1


class AcquireState(Enum):
    Stop = 0
    Single = 1
    Continuous = 2


class Blanker(Enum):
    Off = 0
    On = 1
    Acq = 2


class PixelCount(Enum):
    X128 = 128
    X256 = 256
    X512 = 512
    X1024 = 1024
    X2048 = 2048
    X4096 = 4096
    X8192 = 8192
    Custom = 0


class StemAdjust(Enum):
    COMB = 0
    EL = 1
    MECH = 2


class DiffAdjust(Enum):
    ZOOM = 0
    EL = 1
    MECH = 2
    AP = 3


class EleTilt(Enum):
    Illumination = 0
    Projection = 1


class EleAdjust(Enum):
    Scan = 0
    Precession = 1


class AxisDirection(Enum):
    X = 0
    Y = 1


class FocusType(Enum):
    C3 = 0
    OB = 1


class ROIMode(Enum):
    Full = 0
    ROI256 = 1
    ROI128 = 2


class PrecessMetric(Enum):
    Gabor = 0
    Sobel = 1


class DeprecessMetric(Enum):
    AISpot = 0
    Sobel = 1
