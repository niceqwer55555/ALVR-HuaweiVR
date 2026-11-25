package com.generic.alvr;

import android.app.Activity;
import android.content.Context;
import android.hardware.Sensor;
import android.hardware.SensorEvent;
import android.hardware.SensorEventListener;
import android.hardware.SensorManager;
import android.media.MediaCodecInfo;
import android.media.MediaCodecList;
import android.os.BatteryManager;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.os.PowerManager;
import android.util.Log;
import android.view.Gravity;
import android.view.SurfaceHolder;
import android.view.SurfaceView;
import android.view.View;
import android.view.WindowManager;
import android.widget.FrameLayout;
import android.widget.TextView;

import java.net.DatagramPacket;
import java.net.DatagramSocket;
import java.net.InetAddress;
import java.util.ArrayList;
import java.util.List;
import java.util.Locale;

/**
 * 通用 Android 平台桥 / 诊断壳。
 *
 * 职责边界：
 *  - 把手机 IMU 送入 native Mahony 姿态核心；
 *  - 接收 localhost:47110 的外部姿态注入；
 *  - 枚举高通 H.264 硬解并显示设备/手柄状态；
 *  - 提供一个 SurfaceView 作为后续 OpenXR/解码器渲染的挂载点。
 *
 * 这不是完整 ALVR 串流客户端：没有实现 ALVR 控制协议、视频解码和双目渲染。
 * 完整客户端需要 client_core + OpenXR runtime，见 README 的说明。
 */
public class AlvrActivity extends Activity implements SensorEventListener {
    private static final String TAG = "GenericAlvr";
    private static final int VIEW_W = 1600;
    private static final int VIEW_H = 1600;
    private static final int REFRESH_HZ = 72;
    private static final int POSE_UDP_PORT = 47110;

    private SensorManager sensorManager;
    private Sensor gyro;
    private Sensor accel;
    private Sensor proximity;
    private SurfaceView surfaceView;
    private TextView statusText;
    private TextView ledHeadset;
    private TextView ledLeft;
    private TextView ledRight;

    private Handler uiHandler;
    private volatile boolean running = true;
    private Thread udpThread;
    private PowerManager.WakeLock wakeLock;

    static {
        System.loadLibrary("generic_alvr_bridge");
    }

    private native void nativeInit(int viewW, int viewH, int refreshHz, boolean positionTracking);
    private native void nativeFeedImu(int type, float x, float y, float z, long tNanos);
    private native boolean nativeFeedPoseText(String text);
    private native void nativeFeedControllerImu(int hand, float gx, float gy, float gz,
                                                 float ax, float ay, float az, long tNanos);
    private native void nativeFeedControllerInput(int hand, int buttons, float ax, float ay,
                                                  float trigger, float grip);
    private native void nativeSetControllerConnected(int hand, boolean connected);
    private native void nativeGetHeadPose(float[] out);
    private native boolean nativeHeadPosValid();
    private native void nativeRecenter();
    private native String[] nativeDecoderCandidates();
    private native boolean nativeFeedHidInput(byte[] data);
    private native void nativeSetDeviceStatus(int batteryPercent, boolean proximityNear,
                                              boolean validBattery, boolean validProximity);
    private native void nativeGetControllerStatus(int hand, int[] out);
    private native void nativeSetControllerBattery(int hand, int batteryPercent);
    private native void nativeGetDeviceStatus(int[] out);

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON
                | WindowManager.LayoutParams.FLAG_FULLSCREEN);
        setRequestedOrientation(android.content.pm.ActivityInfo.SCREEN_ORIENTATION_LANDSCAPE);

        uiHandler = new Handler(Looper.getMainLooper());
        sensorManager = (SensorManager) getSystemService(Context.SENSOR_SERVICE);
        gyro = sensorManager.getDefaultSensor(Sensor.TYPE_GYROSCOPE);
        accel = sensorManager.getDefaultSensor(Sensor.TYPE_ACCELEROMETER);
        proximity = sensorManager.getDefaultSensor(Sensor.TYPE_PROXIMITY);

        PowerManager pm = (PowerManager) getSystemService(Context.POWER_SERVICE);
        wakeLock = pm.newWakeLock(PowerManager.SCREEN_DIM_WAKE_LOCK, TAG);
        wakeLock.setReferenceCounted(false);
        wakeLock.acquire();

        buildUi();
        nativeInit(VIEW_W, VIEW_H, REFRESH_HZ, true);
        Log.i(TAG, "nativeInit " + VIEW_W + "x" + VIEW_H + " @" + REFRESH_HZ);

        startPoseUdpReceiver();
        uiHandler.postDelayed(this::refreshStatus, 300);
    }

    private void buildUi() {
        FrameLayout root = new FrameLayout(this);
        root.setBackgroundColor(0xff000000);

        surfaceView = new SurfaceView(this);
        surfaceView.getHolder().addCallback(new SurfaceHolder.Callback() {
            @Override public void surfaceCreated(SurfaceHolder holder) {
                Log.i(TAG, "Surface created " + holder.getSurfaceFrame().width()
                        + "x" + holder.getSurfaceFrame().height());
            }
            @Override public void surfaceChanged(SurfaceHolder holder, int format, int width, int height) {
                Log.i(TAG, "Surface changed " + width + "x" + height);
            }
            @Override public void surfaceDestroyed(SurfaceHolder holder) { }
        });
        root.addView(surfaceView, new FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT));

        statusText = new TextView(this);
        statusText.setTextColor(0xffd0d0d0);
        statusText.setTextSize(12);
        statusText.setPadding(18, 18, 18, 18);
        FrameLayout.LayoutParams statusLp = new FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.WRAP_CONTENT, FrameLayout.LayoutParams.WRAP_CONTENT,
                Gravity.TOP | Gravity.START);
        root.addView(statusText, statusLp);

        ledHeadset = makeLed(0xff4ade80);
        ledLeft = makeLed(0xff888888);
        ledRight = makeLed(0xff888888);
        FrameLayout.LayoutParams headsetLp = new FrameLayout.LayoutParams(dp(12), dp(12),
                Gravity.TOP | Gravity.END);
        headsetLp.setMargins(0, dp(18), dp(18), 0);
        FrameLayout.LayoutParams leftLp = new FrameLayout.LayoutParams(dp(12), dp(12),
                Gravity.TOP | Gravity.END);
        leftLp.setMargins(0, dp(38), dp(18), 0);
        FrameLayout.LayoutParams rightLp = new FrameLayout.LayoutParams(dp(12), dp(12),
                Gravity.TOP | Gravity.END);
        rightLp.setMargins(0, dp(58), dp(18), 0);
        root.addView(ledHeadset, headsetLp);
        root.addView(ledLeft, leftLp);
        root.addView(ledRight, rightLp);

        surfaceView.setOnClickListener(v -> {
            nativeRecenter();
            Log.i(TAG, "recenter");
        });

        setContentView(root);
    }

    private TextView makeLed(int color) {
        TextView v = new TextView(this);
        v.setBackgroundColor(color);
        return v;
    }

    private int dp(int value) {
        return Math.round(value * getResources().getDisplayMetrics().density);
    }

    private void startPoseUdpReceiver() {
        udpThread = new Thread(() -> {
            try (DatagramSocket socket = new DatagramSocket(POSE_UDP_PORT,
                    InetAddress.getByName("127.0.0.1"))) {
                socket.setReuseAddress(true);
                byte[] buf = new byte[512];
                Log.i(TAG, "pose UDP listening on 127.0.0.1:" + POSE_UDP_PORT);
                while (running) {
                    DatagramPacket packet = new DatagramPacket(buf, buf.length);
                    socket.receive(packet);
                    String text = new String(packet.getData(), 0, packet.getLength(),
                            java.nio.charset.StandardCharsets.UTF_8).trim();
                    if (!text.isEmpty()) {
                        nativeFeedPoseText(text);
                    }
                }
            } catch (Exception e) {
                if (running) Log.w(TAG, "pose UDP stopped: " + e.getMessage());
            }
        }, "GenericAlvrPoseUdp");
        udpThread.setDaemon(true);
        udpThread.start();
    }

    @Override
    protected void onResume() {
        super.onResume();
        if (gyro != null) sensorManager.registerListener(this, gyro, SensorManager.SENSOR_DELAY_GAME);
        if (accel != null) sensorManager.registerListener(this, accel, SensorManager.SENSOR_DELAY_GAME);
        if (proximity != null) sensorManager.registerListener(this, proximity, SensorManager.SENSOR_DELAY_NORMAL);
    }

    @Override
    protected void onPause() {
        sensorManager.unregisterListener(this);
        super.onPause();
    }

    @Override
    protected void onDestroy() {
        running = false;
        if (udpThread != null) udpThread.interrupt();
        if (wakeLock != null && wakeLock.isHeld()) wakeLock.release();
        nativeRecenter();
        super.onDestroy();
    }

    @Override
    public void onSensorChanged(SensorEvent event) {
        int type = event.sensor.getType();
        if (type == Sensor.TYPE_GYROSCOPE) {
            nativeFeedImu(0, event.values[0], event.values[1], event.values[2], event.timestamp);
        } else if (type == Sensor.TYPE_ACCELEROMETER) {
            nativeFeedImu(1, event.values[0], event.values[1], event.values[2], event.timestamp);
        } else if (type == Sensor.TYPE_PROXIMITY) {
            boolean near = event.values.length > 0 && event.values[0] < event.sensor.getMaximumRange();
            nativeSetDeviceStatus(readBatteryPercent(), near, true, true);
        }
    }

    @Override
    public void onAccuracyChanged(Sensor sensor, int accuracy) { }

    private int readBatteryPercent() {
        BatteryManager bm = (BatteryManager) getSystemService(Context.BATTERY_SERVICE);
        return bm.getIntProperty(BatteryManager.BATTERY_PROPERTY_CAPACITY);
    }

    private void refreshStatus() {
        if (!running) return;

        float[] pose = new float[7];
        nativeGetHeadPose(pose);
        boolean posValid = nativeHeadPosValid();

        int[] dev = new int[7];
        nativeGetDeviceStatus(dev);
        int[] left = new int[3];
        int[] right = new int[3];
        nativeGetControllerStatus(0, left);
        nativeGetControllerStatus(1, right);

        String decoders = android.text.TextUtils.join(", ", availableDecoders());
        String status = String.format(Locale.US,
                "HUAWEI VR Glass / Generic ALVR bridge\n"
                + "view: %dx%d @ %dHz  pose: %s\n"
                + "quat: %.3f %.3f %.3f %.3f\n"
                + "pos: %.3f %.3f %.3f  %s\n"
                + "battery: %d%%  proximity: %s\n"
                + "headset: %s  left: %s  right: %s\n"
                + "decoder: %s",
                VIEW_W, VIEW_H, REFRESH_HZ, posValid ? "6DoF" : "3DoF",
                pose[3], pose[0], pose[1], pose[2],
                pose[4], pose[5], pose[6], posValid ? "valid" : "invalid",
                dev[2], dev[0] != 0 ? "near" : "far",
                dev[4] != 0 ? "on" : "off",
                controllerLabel(left), controllerLabel(right),
                decoders.isEmpty() ? "none" : decoders);

        statusText.setText(status);
        ledHeadset.setBackgroundColor(dev[4] != 0 ? 0xff4ade80 : 0xff888888);
        ledLeft.setBackgroundColor(left[0] != 0 ? 0xff4ade80 : 0xff888888);
        ledRight.setBackgroundColor(right[0] != 0 ? 0xff4ade80 : 0xff888888);

        if (left[2] >= 0) nativeSetControllerBattery(0, left[2]);
        if (right[2] >= 0) nativeSetControllerBattery(1, right[2]);

        uiHandler.postDelayed(this::refreshStatus, 500);
    }

    private String controllerLabel(int[] c) {
        if (c[0] == 0) return "off";
        return (c[1] == 6 ? "6DoF" : "3DoF") + (c[2] >= 0 ? " " + c[2] + "%" : "");
    }

    private List<String> availableDecoders() {
        List<String> result = new ArrayList<>();
        String[] wanted = nativeDecoderCandidates();
        if (wanted == null) return result;
        MediaCodecList list = new MediaCodecList(MediaCodecList.REGULAR_CODECS);
        for (MediaCodecInfo info : list.getCodecInfos()) {
            if (!info.isEncoder()) {
                for (String name : wanted) {
                    if (info.getName().equalsIgnoreCase(name)) result.add(name);
                }
            }
        }
        return result;
    }
}
