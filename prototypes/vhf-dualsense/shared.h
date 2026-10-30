#pragma once
#include <cstdint>

inline constexpr GUID kControlInterface =
    {0x3a04ce2a, 0xb6ba, 0x4f17, {0x83, 0x42, 0xd8, 0x94, 0xe6, 0x61, 0x44, 0x71}};
inline constexpr DWORD kIoctlCreate = CTL_CODE(FILE_DEVICE_UNKNOWN, 0x800, METHOD_BUFFERED, FILE_WRITE_ACCESS);
inline constexpr DWORD kIoctlInput = CTL_CODE(FILE_DEVICE_UNKNOWN, 0x801, METHOD_BUFFERED, FILE_WRITE_ACCESS);
inline constexpr DWORD kIoctlStatus = CTL_CODE(FILE_DEVICE_UNKNOWN, 0x802, METHOD_BUFFERED, FILE_READ_ACCESS);
inline constexpr DWORD kIoctlDestroy = CTL_CODE(FILE_DEVICE_UNKNOWN, 0x803, METHOD_BUFFERED, FILE_WRITE_ACCESS);
inline constexpr size_t kMaxDescriptor = 4096;
inline constexpr size_t kMaxReport = 128;
inline constexpr size_t kMaxFeatures = 16;

struct CreateRequest {
    uint32_t version;
    uint16_t vid, pid, revision, descriptor_length;
    uint16_t input_length, output_length, feature_length, reserved;
    uint8_t descriptor[kMaxDescriptor];
    uint8_t features[kMaxFeatures][kMaxReport];
    uint16_t feature_lengths[kMaxFeatures];
};
struct DriverStatus {
    uint32_t version, active, submitted, get_input;
    uint32_t get_feature, set_feature, output, unsupported_feature;
    uint32_t last_feature_id, last_submit_status, watchdog_resets, last_set_feature;
};
static_assert(sizeof(CreateRequest) == 6196);
static_assert(sizeof(DriverStatus) == 48);
