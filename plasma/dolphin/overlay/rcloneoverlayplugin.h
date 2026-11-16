#pragma once

#include <KOverlayIconPlugin>

#include <QHash>
#include <QSet>
#include <QStringList>
#include <QTimer>

class DaemonClient;

class RcloneOverlayPlugin : public KOverlayIconPlugin
{
    Q_OBJECT
    Q_PLUGIN_METADATA(IID "org.kde.overlayicon.rclone" FILE "rcloneoverlayplugin.json")
public:
    explicit RcloneOverlayPlugin(QObject *parent = nullptr);

    QStringList getOverlays(const QUrl &item) override;

private:
    void queue(const QString &path);
    void flush();
    void onReply(const QMap<QString, QStringList> &states);
    void store(const QString &path, const QStringList &emblems);

    DaemonClient *m_client;
    QHash<QString, QStringList> m_cache;
    QSet<QString> m_pending; // waiting for the batch timer
    QSet<QString> m_inFlight; // sent, no reply yet
    QTimer m_flushTimer;
};
