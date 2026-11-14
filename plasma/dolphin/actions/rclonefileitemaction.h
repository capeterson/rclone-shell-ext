#pragma once

#include <KAbstractFileItemActionPlugin>

#include <QSet>
#include <QString>

class DaemonClient;

class RcloneFileItemAction : public KAbstractFileItemActionPlugin
{
    Q_OBJECT
public:
    RcloneFileItemAction(QObject *parent, const QVariantList &args);

    QList<QAction *> actions(const KFileItemListProperties &fileItemInfos, QWidget *parentWidget) override;

private:
    void refreshPins();

    DaemonClient *m_client;
    QSet<QString> m_pins; // mount-relative paths
    bool m_pinsKnown = false;
};
