cmake_minimum_required(VERSION 3.20)
project(rclone-shell-ext-dolphin VERSION 0.1.0 LANGUAGES CXX)

set(QT_MIN_VERSION "6.6.0")
set(KF_MIN_VERSION "6.0.0")

find_package(ECM ${KF_MIN_VERSION} REQUIRED NO_MODULE)
set(CMAKE_MODULE_PATH ${ECM_MODULE_PATH})

include(KDEInstallDirs)
include(KDECMakeSettings)
include(KDECompilerSettings NO_POLICY_SCOPE)

find_package(Qt6 ${QT_MIN_VERSION} REQUIRED COMPONENTS Core DBus Widgets)
find_package(KF6 ${KF_MIN_VERSION} REQUIRED COMPONENTS KIO CoreAddons)
find_package(KF6I18n ${KF_MIN_VERSION} QUIET)

add_definitions(-DTRANSLATION_DOMAIN="rclone-shell-ext")

# Shared async D-Bus client (static, linked into both plugins).
add_library(rclonedaemonclient STATIC common/daemonclient.cpp common/daemonclient.h)
set_target_properties(rclonedaemonclient PROPERTIES POSITION_INDEPENDENT_CODE ON)
target_include_directories(rclonedaemonclient PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}/common)
target_link_libraries(rclonedaemonclient PUBLIC Qt6::Core Qt6::DBus)

kcoreaddons_add_plugin(rcloneoverlayplugin
    SOURCES overlay/rcloneoverlayplugin.cpp overlay/rcloneoverlayplugin.h
    INSTALL_NAMESPACE "kf6/overlayicon")
target_link_libraries(rcloneoverlayplugin PRIVATE rclonedaemonclient
    Qt6::Core Qt6::DBus KF6::KIOCore)

kcoreaddons_add_plugin(rclonefileitemaction
    SOURCES actions/rclonefileitemaction.cpp actions/rclonefileitemaction.h
    INSTALL_NAMESPACE "kf6/kfileitemaction")
target_link_libraries(rclonefileitemaction PRIVATE rclonedaemonclient
    Qt6::Core Qt6::DBus Qt6::Widgets KF6::KIOCore KF6::KIOWidgets KF6::CoreAddons)
if(KF6I18n_FOUND)
    target_link_libraries(rclonefileitemaction PRIVATE KF6::I18n)
else()
    message(WARNING "KF6I18n not found: strings will not be translatable")
    target_compile_definitions(rclonefileitemaction PRIVATE RCLONE_NO_KI18N)
endif()
